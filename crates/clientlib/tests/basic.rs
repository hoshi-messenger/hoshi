mod common;

use std::time::Duration;

use base64::{Engine as _, engine::general_purpose::STANDARD};
use common::with_control_plane_and_relay;
use hoshi_clientlib::{ClientConfig, ClientConnection};
use hoshi_control_plane::{
    Client, ClientType,
    api::{NoisePublicKeyResponse, RegisterClientRequest, RelayEntry},
};
use serde::Serialize;
use tempfile::TempDir;

const REGISTRATION_NOISE_PATTERN: &str = "Noise_X_25519_ChaChaPoly_BLAKE2s";

#[derive(Debug, Serialize)]
struct ClientRegistrationProofPayload<'a> {
    public_key: &'a str,
    owner_id: Option<&'a str>,
    client_type: &'a ClientType,
}

#[tokio::test]
async fn config_load_creates_default_file_when_missing() {
    let dir = TempDir::new().expect("temp dir");
    let config_path = dir.path().join(".hoshi/client.toml");

    let config = ClientConfig::load_from_path(&config_path).expect("load client config");
    assert!(config_path.exists(), "config file should be created");
    assert_eq!(config.config_path, config_path);

    let raw = std::fs::read_to_string(&config_path).expect("read config");
    let value = raw.parse::<toml::Value>().expect("parse toml");
    let table = value.as_table().expect("table");

    let uri = table
        .get("control_plane_uri")
        .and_then(toml::Value::as_str)
        .expect("control_plane_uri");
    assert!(!uri.is_empty());

    let key = table
        .get("noise_static_private_key")
        .and_then(toml::Value::as_str)
        .expect("noise_static_private_key");
    let decoded = STANDARD.decode(key).expect("base64 decode");
    assert_eq!(decoded.len(), 32);
}

#[tokio::test]
async fn config_load_normalizes_uri_and_key() {
    let dir = TempDir::new().expect("temp dir");
    let config_path = dir.path().join("client.toml");
    let key = STANDARD.encode([3_u8; 32]);
    let raw = format!(
        r#"
control_plane_uri = "  http://127.0.0.1:2600  "
noise_static_private_key = "  {key}  "
"#
    );
    std::fs::write(&config_path, raw).expect("write config");

    let config = ClientConfig::load_from_path(&config_path).expect("load config");
    assert_eq!(config.control_plane_uri, "http://127.0.0.1:2600");
    assert_eq!(config.noise_static_private_key, key);
}

#[tokio::test]
async fn connect_fails_for_unknown_client_key() {
    with_control_plane_and_relay(|control_plane, _relay| async move {
        let cp_uri = control_plane.config.uri();
        let (_public_key, private_key) = generate_noise_keypair();
        let (_config_dir, config_path) = write_client_config(&cp_uri, &private_key);
        let config = ClientConfig::load_from_path(&config_path).expect("load config");

        let err = match ClientConnection::connect_with_config(config).await {
            Ok(_) => panic!("connect should fail for unknown client"),
            Err(err) => err,
        };
        assert!(
            err.to_string().contains("unknown client"),
            "unexpected error: {err:#}"
        );
    })
    .await;
}

#[tokio::test]
async fn connect_to_stack_succeeds_for_registered_client() {
    with_control_plane_and_relay(|control_plane, _relay| async move {
        let cp_uri = control_plane.config.uri();
        let http = reqwest::Client::new();

        let (sender_public_key, sender_private_key) = generate_noise_keypair();
        let sender = register_client(
            &http,
            &cp_uri,
            &sender_public_key,
            &sender_private_key,
            None,
            ClientType::Device,
        )
        .await;

        let _relay = wait_for_registered_relay(&http, &cp_uri).await;

        let (_sender_config_dir, sender_config_path) =
            write_client_config(&cp_uri, &sender_private_key);
        let sender_config =
            ClientConfig::load_from_path(&sender_config_path).expect("load sender config");
        let mut sender_connection = ClientConnection::connect_with_config(sender_config)
            .await
            .expect("connect sender");

        assert_eq!(sender_connection.device_guid(), sender.id);
        assert_eq!(sender_connection.client_guid(), sender.id);
        assert!(!sender_connection.relay_guid().is_empty());

        sender_connection.close().await.expect("close sender");
    })
    .await;
}

#[tokio::test]
async fn send_message_to_self_roundtrips_via_relay() {
    with_control_plane_and_relay(|control_plane, _relay| async move {
        let cp_uri = control_plane.config.uri();
        let http = reqwest::Client::new();

        let (public_key, private_key) = generate_noise_keypair();
        let registered = register_client(
            &http,
            &cp_uri,
            &public_key,
            &private_key,
            None,
            ClientType::Device,
        )
        .await;

        let _relay = wait_for_registered_relay(&http, &cp_uri).await;

        let (_config_dir, config_path) = write_client_config(&cp_uri, &private_key);
        let config = ClientConfig::load_from_path(&config_path).expect("load config");
        let mut connection = ClientConnection::connect_with_config(config)
            .await
            .expect("connect client");

        let self_guid = connection.device_guid().to_string();
        assert_eq!(self_guid, registered.id);

        connection
            .send_text(&self_guid, "self-loopback-message")
            .await
            .expect("send to self");

        let received = tokio::time::timeout(Duration::from_secs(2), connection.receive_message())
            .await
            .expect("timeout waiting for self-loop message")
            .expect("receive self-loop message");
        assert_eq!(received.recipient, self_guid);
        assert_eq!(received.payload, b"self-loopback-message");

        connection.close().await.expect("close connection");
    })
    .await;
}

#[tokio::test]
async fn connect_does_not_persist_jwt_in_config() {
    with_control_plane_and_relay(|control_plane, _relay| async move {
        let cp_uri = control_plane.config.uri();
        let http = reqwest::Client::new();

        let (sender_public_key, sender_private_key) = generate_noise_keypair();
        let _sender = register_client(
            &http,
            &cp_uri,
            &sender_public_key,
            &sender_private_key,
            None,
            ClientType::Device,
        )
        .await;

        let _relay = wait_for_registered_relay(&http, &cp_uri).await;

        let (_config_dir, config_path) = write_client_config(&cp_uri, &sender_private_key);
        let config = ClientConfig::load_from_path(&config_path).expect("load config");
        let mut connection = ClientConnection::connect_with_config(config)
            .await
            .expect("connect sender");
        connection.close().await.expect("close connection");

        let raw = std::fs::read_to_string(&config_path).expect("read config");
        let value = raw.parse::<toml::Value>().expect("parse toml");
        let table = value.as_table().expect("table");
        assert_eq!(table.len(), 2);
        assert!(table.contains_key("control_plane_uri"));
        assert!(table.contains_key("noise_static_private_key"));
        assert!(!table.contains_key("token"));
        assert!(!table.contains_key("jwt"));
    })
    .await;
}

fn write_client_config(
    control_plane_uri: &str,
    private_key: &[u8; 32],
) -> (TempDir, std::path::PathBuf) {
    let dir = TempDir::new().expect("temp dir");
    let config_path = dir.path().join("client.toml");
    let key = STANDARD.encode(private_key);
    let content = format!(
        r#"
control_plane_uri = "{control_plane_uri}"
noise_static_private_key = "{key}"
"#
    );
    std::fs::write(&config_path, content).expect("write config");
    (dir, config_path)
}

fn generate_noise_keypair() -> (String, [u8; 32]) {
    let params = REGISTRATION_NOISE_PATTERN
        .parse()
        .expect("parse noise pattern");
    let keypair = snow::Builder::new(params)
        .generate_keypair()
        .expect("generate keypair");
    let private_key: [u8; 32] = keypair
        .private
        .as_slice()
        .try_into()
        .expect("private key len");
    (STANDARD.encode(&keypair.public), private_key)
}

fn create_noise_handshake(
    private_key: &[u8; 32],
    remote_public_key_b64: &str,
    payload: &[u8],
) -> String {
    let remote_public_key = STANDARD
        .decode(remote_public_key_b64)
        .expect("decode remote key");
    let params = REGISTRATION_NOISE_PATTERN
        .parse()
        .expect("parse noise pattern");
    let mut initiator = snow::Builder::new(params)
        .local_private_key(private_key)
        .remote_public_key(&remote_public_key)
        .build_initiator()
        .expect("build initiator");

    let mut message = vec![0_u8; payload.len() + 256];
    let message_len = initiator
        .write_message(payload, &mut message)
        .expect("write handshake");
    STANDARD.encode(&message[..message_len])
}

async fn fetch_control_plane_noise_key(
    http: &reqwest::Client,
    control_plane_uri: &str,
) -> NoisePublicKeyResponse {
    let res = http
        .get(format!("{}/noise/public-key", control_plane_uri))
        .send()
        .await
        .expect("noise endpoint");
    assert!(res.status().is_success());
    res.json::<NoisePublicKeyResponse>()
        .await
        .expect("noise payload")
}

async fn register_client(
    http: &reqwest::Client,
    control_plane_uri: &str,
    public_key: &str,
    private_key: &[u8; 32],
    owner_id: Option<&str>,
    client_type: ClientType,
) -> Client {
    let noise = fetch_control_plane_noise_key(http, control_plane_uri).await;
    let proof_payload = serde_json::to_vec(&ClientRegistrationProofPayload {
        public_key,
        owner_id,
        client_type: &client_type,
    })
    .expect("serialize client proof");

    let req = RegisterClientRequest {
        public_key: public_key.to_string(),
        owner_id: owner_id.map(str::to_string),
        client_type,
        noise_handshake: create_noise_handshake(private_key, &noise.public_key, &proof_payload),
    };
    let res = http
        .post(format!("{}/clients", control_plane_uri))
        .json(&req)
        .send()
        .await
        .expect("register client");
    assert_eq!(res.status(), reqwest::StatusCode::CREATED);
    res.json::<Client>().await.expect("client")
}

async fn wait_for_registered_relay(http: &reqwest::Client, control_plane_uri: &str) -> RelayEntry {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    loop {
        let res = http
            .get(format!("{}/relays", control_plane_uri))
            .send()
            .await
            .expect("list relays");
        assert!(res.status().is_success());
        let relays = res.json::<Vec<RelayEntry>>().await.expect("relay list");
        if let Some(relay) = relays.into_iter().next() {
            return relay;
        }

        assert!(
            tokio::time::Instant::now() < deadline,
            "relay registration did not complete in time"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}
