mod common;

use std::time::Duration;

use base64::{Engine as _, engine::general_purpose::STANDARD};
use common::with_control_plane_and_relay;
use hoshi_clientlib::{ClientConfig, ClientConnection, ClientSession, UserAuthState};
use hoshi_protocol::control_plane::{
    ClientEntry as Client, ClientType, NoisePublicKeyResponse, RegisterClientRequest, RelayEntry,
};
use serde::Serialize;
use tempfile::TempDir;

const REGISTRATION_NOISE_PATTERN: &str = "Noise_X_25519_ChaChaPoly_BLAKE2s";

#[derive(Debug, Serialize)]
struct ClientRegistrationProofPayload<'a> {
    public_key: &'a str,
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
        .get("device_noise_static_private_key")
        .and_then(toml::Value::as_str)
        .expect("device_noise_static_private_key");
    let decoded = STANDARD.decode(key).expect("base64 decode");
    assert_eq!(decoded.len(), 32);

    assert!(table.get("user_noise_static_private_key").is_none());
    assert!(table.get("noise_static_private_key").is_none());
}

#[tokio::test]
async fn config_load_normalizes_uri_and_keys() {
    let dir = TempDir::new().expect("temp dir");
    let config_path = dir.path().join("client.toml");
    let device_key = STANDARD.encode([3_u8; 32]);
    let user_key = STANDARD.encode([7_u8; 32]);
    let raw = format!(
        r#"
control_plane_uri = "  http://127.0.0.1:2600  "
device_noise_static_private_key = "  {device_key}  "
user_noise_static_private_key = "  {user_key}  "
"#
    );
    std::fs::write(&config_path, raw).expect("write config");

    let config = ClientConfig::load_from_path(&config_path).expect("load config");
    assert_eq!(config.control_plane_uri, "http://127.0.0.1:2600");
    assert_eq!(config.device_noise_static_private_key, device_key);
    assert_eq!(
        config.user_noise_static_private_key.as_deref(),
        Some(user_key.as_str())
    );
}

#[tokio::test]
async fn config_load_rejects_legacy_noise_static_private_key_field() {
    let dir = TempDir::new().expect("temp dir");
    let config_path = dir.path().join("client.toml");
    let legacy_key = STANDARD.encode([9_u8; 32]);
    let raw = format!(
        r#"
control_plane_uri = "http://127.0.0.1:2600"
noise_static_private_key = "{legacy_key}"
"#
    );
    std::fs::write(&config_path, raw).expect("write legacy config");

    let err = ClientConfig::load_from_path(&config_path).expect_err("legacy config should fail");
    assert!(
        err.to_string().contains("noise_static_private_key"),
        "unexpected error: {err:#}"
    );
}

#[tokio::test]
async fn connect_auto_registers_device_when_unknown() {
    with_control_plane_and_relay(|control_plane, _relay| async move {
        let cp_uri = control_plane.config.uri();
        let http = reqwest::Client::new();
        let (_public_key, private_key) = generate_noise_keypair();
        let _relay = wait_for_registered_relay(&http, &cp_uri).await;
        let (_config_dir, config_path) = write_client_config(&cp_uri, &private_key, None);
        let config = ClientConfig::load_from_path(&config_path).expect("load config");

        let mut connection = ClientConnection::connect_with_config(config)
            .await
            .expect("connect should auto-register device");
        assert_eq!(connection.client_type(), ClientType::Device);
        assert!(!connection.relay_guid().is_empty());

        let lookup = http
            .get(format!("{}/clients/{}", cp_uri, connection.guid()))
            .send()
            .await
            .expect("lookup device guid");
        assert_eq!(lookup.status(), reqwest::StatusCode::OK);

        connection.close().await.expect("close connection");
    })
    .await;
}

#[tokio::test]
async fn connect_to_stack_succeeds_for_registered_device() {
    with_control_plane_and_relay(|control_plane, _relay| async move {
        let cp_uri = control_plane.config.uri();
        let http = reqwest::Client::new();

        let (sender_public_key, sender_private_key) = generate_noise_keypair();
        let sender = register_client(
            &http,
            &cp_uri,
            &sender_public_key,
            &sender_private_key,
            ClientType::Device,
        )
        .await;

        let _relay = wait_for_registered_relay(&http, &cp_uri).await;

        let (_sender_config_dir, sender_config_path) =
            write_client_config(&cp_uri, &sender_private_key, None);
        let sender_config =
            ClientConfig::load_from_path(&sender_config_path).expect("load sender config");
        let mut sender_connection = ClientConnection::connect_with_config(sender_config)
            .await
            .expect("connect sender");

        assert_eq!(sender_connection.guid(), sender.id);
        assert_eq!(sender_connection.client_type(), ClientType::Device);
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
            ClientType::Device,
        )
        .await;

        let _relay = wait_for_registered_relay(&http, &cp_uri).await;

        let (_config_dir, config_path) = write_client_config(&cp_uri, &private_key, None);
        let config = ClientConfig::load_from_path(&config_path).expect("load config");
        let mut connection = ClientConnection::connect_with_config(config)
            .await
            .expect("connect client");

        let self_guid = connection.guid().to_string();
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
async fn client_session_reports_no_local_user_identity_when_missing() {
    with_control_plane_and_relay(|control_plane, _relay| async move {
        let cp_uri = control_plane.config.uri();
        let http = reqwest::Client::new();
        let (device_public_key, device_private_key) = generate_noise_keypair();
        let _device = register_client(
            &http,
            &cp_uri,
            &device_public_key,
            &device_private_key,
            ClientType::Device,
        )
        .await;

        let (_config_dir, config_path) = write_client_config(&cp_uri, &device_private_key, None);
        let config = ClientConfig::load_from_path(&config_path).expect("load config");
        let mut session = ClientSession::connect_with_config(config)
            .await
            .expect("connect session");

        assert_eq!(session.user_auth_state(), UserAuthState::NoLocalIdentity);
        assert!(session.user_connection().is_none());
        assert_eq!(
            session.device_connection().client_type(),
            ClientType::Device
        );

        session.close().await.expect("close session");
    })
    .await;
}

#[tokio::test]
async fn client_session_reports_unknown_user_identity_without_failing() {
    with_control_plane_and_relay(|control_plane, _relay| async move {
        let cp_uri = control_plane.config.uri();
        let http = reqwest::Client::new();
        let (device_public_key, device_private_key) = generate_noise_keypair();
        let _device = register_client(
            &http,
            &cp_uri,
            &device_public_key,
            &device_private_key,
            ClientType::Device,
        )
        .await;

        let (_unknown_user_public_key, unknown_user_private_key) = generate_noise_keypair();
        let (_config_dir, config_path) = write_client_config(
            &cp_uri,
            &device_private_key,
            Some(&unknown_user_private_key),
        );
        let config = ClientConfig::load_from_path(&config_path).expect("load config");
        let mut session = ClientSession::connect_with_config(config)
            .await
            .expect("connect session");

        assert_eq!(session.user_auth_state(), UserAuthState::UnknownIdentity);
        assert!(session.user_connection().is_none());
        assert_eq!(
            session.device_connection().client_type(),
            ClientType::Device
        );

        session.close().await.expect("close session");
    })
    .await;
}

#[tokio::test]
async fn client_session_connects_device_and_user_on_same_relay() {
    with_control_plane_and_relay(|control_plane, _relay| async move {
        let cp_uri = control_plane.config.uri();
        let http = reqwest::Client::new();
        let (device_public_key, device_private_key) = generate_noise_keypair();
        let _device = register_client(
            &http,
            &cp_uri,
            &device_public_key,
            &device_private_key,
            ClientType::Device,
        )
        .await;
        let (user_public_key, user_private_key) = generate_noise_keypair();
        let _user = register_client(
            &http,
            &cp_uri,
            &user_public_key,
            &user_private_key,
            ClientType::User,
        )
        .await;

        let (_config_dir, config_path) =
            write_client_config(&cp_uri, &device_private_key, Some(&user_private_key));
        let config = ClientConfig::load_from_path(&config_path).expect("load config");
        let mut session = ClientSession::connect_with_config(config)
            .await
            .expect("connect session");

        assert_eq!(session.user_auth_state(), UserAuthState::Connected);
        let user_connection = session
            .user_connection()
            .expect("user connection should be present");
        assert_eq!(user_connection.client_type(), ClientType::User);
        assert_eq!(
            user_connection.relay_guid(),
            session.device_connection().relay_guid()
        );
        assert_eq!(
            session.relay_guid(),
            session.device_connection().relay_guid()
        );

        session.close().await.expect("close session");
    })
    .await;
}

#[tokio::test]
async fn connect_does_not_persist_jwt_in_config() {
    with_control_plane_and_relay(|control_plane, _relay| async move {
        let cp_uri = control_plane.config.uri();
        let http = reqwest::Client::new();
        let (_sender_public_key, sender_private_key) = generate_noise_keypair();
        let _relay = wait_for_registered_relay(&http, &cp_uri).await;

        let (_config_dir, config_path) = write_client_config(&cp_uri, &sender_private_key, None);
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
        assert!(table.contains_key("device_noise_static_private_key"));
        assert!(!table.contains_key("token"));
        assert!(!table.contains_key("jwt"));
    })
    .await;
}

fn write_client_config(
    control_plane_uri: &str,
    device_private_key: &[u8; 32],
    user_private_key: Option<&[u8; 32]>,
) -> (TempDir, std::path::PathBuf) {
    let dir = TempDir::new().expect("temp dir");
    let config_path = dir.path().join("client.toml");
    let device_key = STANDARD.encode(device_private_key);
    let mut content = format!(
        r#"
control_plane_uri = "{control_plane_uri}"
device_noise_static_private_key = "{device_key}"
"#
    );

    if let Some(user_private_key) = user_private_key {
        let user_key = STANDARD.encode(user_private_key);
        content.push_str(&format!("user_noise_static_private_key = \"{user_key}\"\n"));
    }

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
    client_type: ClientType,
) -> Client {
    let noise = fetch_control_plane_noise_key(http, control_plane_uri).await;
    let proof_payload = serde_json::to_vec(&ClientRegistrationProofPayload {
        public_key,
        client_type: &client_type,
    })
    .expect("serialize client proof");

    let req = RegisterClientRequest {
        public_key: public_key.to_string(),
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
