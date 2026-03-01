mod common;

use std::time::Duration;

use base64::{Engine as _, engine::general_purpose::STANDARD};
use common::with_control_plane_and_relay;
use hoshi_clientlib::{ClientDatabase, ClientManager};
use hoshi_protocol::control_plane::{
    ClientEntry as Client, ClientType, NoisePublicKeyResponse, RegisterClientRequest,
};
use reqwest::StatusCode;
use serde::Serialize;
use tempfile::TempDir;
use uuid::Uuid;

const REGISTRATION_NOISE_PATTERN: &str = "Noise_X_25519_ChaChaPoly_BLAKE2s";

#[derive(Debug, Serialize)]
struct ClientRegistrationProofPayload<'a> {
    public_key: &'a str,
    client_type: &'a ClientType,
}

#[tokio::test]
async fn database_open_creates_file_and_default_control_plane_uri() {
    let dir = TempDir::new().expect("temp dir");
    let db_path = dir.path().join(".hoshi/client.sqlite3");

    let db = ClientDatabase::open(&db_path)
        .await
        .expect("open client database");
    assert!(db_path.exists(), "db file should be created");
    assert_eq!(db.db_path, db_path);

    let uri = db
        .get_control_plane_uri()
        .await
        .expect("read default control_plane_uri");
    assert!(!uri.is_empty());
}

#[tokio::test]
async fn database_config_key_value_roundtrip() {
    let (_dir, db) = create_test_database().await;

    let value = db.get_config("test").await.expect("read missing config");
    assert!(value.is_none());

    db.set_config("test", b"123")
        .await
        .expect("write binary config");
    let value = db
        .get_config("test")
        .await
        .expect("read binary config")
        .expect("binary config should exist");
    assert_eq!(value, b"123");

    db.set_config_string("test_str", "hello")
        .await
        .expect("write string config");
    let value = db
        .get_config_string("test_str")
        .await
        .expect("read string config");
    assert_eq!(value.as_deref(), Some("hello"));
}

#[tokio::test]
async fn database_key_upsert_roundtrip() {
    let (_dir, db) = create_test_database().await;

    let (_public_key, private_key) = generate_noise_keypair();
    let guid = Uuid::now_v7().to_string().to_uppercase();
    let private_key_b64 = STANDARD.encode(private_key);

    db.upsert_key(&guid, ClientType::Device, &private_key_b64)
        .await
        .expect("upsert key");

    let stored = db
        .get_key(&guid, ClientType::Device)
        .await
        .expect("get key")
        .expect("key should exist");

    assert_eq!(stored.guid, guid.to_lowercase());
    assert_eq!(stored.client_type, ClientType::Device);
    let decoded = STANDARD.decode(&stored.private_key).expect("base64 decode key");
    assert_eq!(decoded.len(), 32);

    db.touch_key(&guid, ClientType::Device)
        .await
        .expect("touch key");
}

#[tokio::test]
async fn database_device_and_user_guid_roundtrip() {
    let (_dir, db) = create_test_database().await;
    let device_guid = Uuid::now_v7().to_string();
    let user_guid = Uuid::now_v7().to_string();

    db.set_device_guid(&device_guid)
        .await
        .expect("set device guid");
    db.set_user_guid(&user_guid).await.expect("set user guid");

    assert_eq!(
        db.get_device_guid().await.expect("get device guid"),
        Some(device_guid)
    );
    assert_eq!(
        db.get_user_guid().await.expect("get user guid"),
        Some(user_guid)
    );

    db.clear_device_guid().await.expect("clear device guid");
    db.clear_user_guid().await.expect("clear user guid");

    assert_eq!(db.get_device_guid().await.expect("get device guid"), None);
    assert_eq!(db.get_user_guid().await.expect("get user guid"), None);
}

#[tokio::test]
async fn connect_from_store_succeeds_for_registered_device() {
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

        let (_db_dir, db) = create_test_database().await;
        db.set_control_plane_uri(&cp_uri)
            .await
            .expect("set control-plane uri");
        db.upsert_key(&registered.id, ClientType::Device, &STANDARD.encode(private_key))
            .await
            .expect("store device key");

        let mut manager = ClientManager::new(db).await.expect("create manager");
        let index = manager
            .connect_from_store(&registered.id, ClientType::Device)
            .await
            .expect("connect stored device");

        assert_eq!(index, 0);
        assert_eq!(manager.connections().len(), 1);
        assert_eq!(
            manager
                .connection(index)
                .expect("device connection")
                .client_type(),
            ClientType::Device
        );

        manager.close_all().await.expect("close manager");
    })
    .await;
}

#[tokio::test]
async fn connect_does_not_auto_register_unknown_identity() {
    with_control_plane_and_relay(|control_plane, _relay| async move {
        let cp_uri = control_plane.config.uri();
        let http = reqwest::Client::new();

        let (_public_key, private_key) = generate_noise_keypair();
        let unknown_guid = Uuid::now_v7().to_string();

        let (_db_dir, db) = create_test_database().await;
        db.set_control_plane_uri(&cp_uri)
            .await
            .expect("set control-plane uri");
        db.upsert_key(&unknown_guid, ClientType::Device, &STANDARD.encode(private_key))
            .await
            .expect("store unknown key");

        let mut manager = ClientManager::new(db).await.expect("create manager");
        let err = manager
            .connect_from_store(&unknown_guid, ClientType::Device)
            .await
            .expect_err("unknown identity should fail");

        assert!(
            err.to_string().contains("unknown client identity"),
            "unexpected error: {err:#}"
        );

        let lookup = http
            .get(format!("{}/clients/{}", cp_uri, unknown_guid))
            .send()
            .await
            .expect("lookup unknown guid");
        assert_eq!(lookup.status(), StatusCode::NOT_FOUND);
    })
    .await;
}

#[tokio::test]
async fn connect_configured_is_best_effort_per_guid() {
    with_control_plane_and_relay(|control_plane, _relay| async move {
        let cp_uri = control_plane.config.uri();
        let http = reqwest::Client::new();

        let (device_public_key, device_private_key) = generate_noise_keypair();
        let registered_device = register_client(
            &http,
            &cp_uri,
            &device_public_key,
            &device_private_key,
            ClientType::Device,
        )
        .await;

        let (_unknown_user_public_key, unknown_user_private_key) = generate_noise_keypair();
        let unknown_user_guid = Uuid::now_v7().to_string();

        let (_db_dir, db) = create_test_database().await;
        db.set_control_plane_uri(&cp_uri)
            .await
            .expect("set control-plane uri");
        db.upsert_key(
            &registered_device.id,
            ClientType::Device,
            &STANDARD.encode(device_private_key),
        )
        .await
        .expect("store device key");
        db.set_device_guid(&registered_device.id)
            .await
            .expect("set device guid");
        db.upsert_key(
            &unknown_user_guid,
            ClientType::User,
            &STANDARD.encode(unknown_user_private_key),
        )
        .await
        .expect("store user key");
        db.set_user_guid(&unknown_user_guid)
            .await
            .expect("set user guid");

        let mut manager = ClientManager::new(db).await.expect("create manager");
        let report = manager
            .connect_configured()
            .await
            .expect("connect configured");

        assert_eq!(report.connected_indices.len(), 1);
        assert_eq!(report.errors.len(), 1);
        assert_eq!(report.errors[0].config_key, "user_guid");
        assert_eq!(report.errors[0].guid, unknown_user_guid);
        assert_eq!(report.errors[0].client_type, ClientType::User);
        assert_eq!(manager.connections().len(), 1);
        assert_eq!(
            manager.connections()[0].client_type(),
            ClientType::Device,
            "device connection should succeed"
        );

        manager.close_all().await.expect("close manager");
    })
    .await;
}

#[tokio::test]
async fn manager_keeps_multiple_open_connections() {
    with_control_plane_and_relay(|control_plane, _relay| async move {
        let cp_uri = control_plane.config.uri();
        let http = reqwest::Client::new();

        let (device_public_key, device_private_key) = generate_noise_keypair();
        let registered_device = register_client(
            &http,
            &cp_uri,
            &device_public_key,
            &device_private_key,
            ClientType::Device,
        )
        .await;

        let (user_public_key, user_private_key) = generate_noise_keypair();
        let registered_user = register_client(
            &http,
            &cp_uri,
            &user_public_key,
            &user_private_key,
            ClientType::User,
        )
        .await;

        let (_db_dir, db) = create_test_database().await;
        db.set_control_plane_uri(&cp_uri)
            .await
            .expect("set control-plane uri");

        let mut manager = ClientManager::new(db).await.expect("create manager");

        let device_index = manager
            .connect(&registered_device.id, &STANDARD.encode(device_private_key))
            .await
            .expect("connect device");
        let user_index = manager
            .connect(&registered_user.id, &STANDARD.encode(user_private_key))
            .await
            .expect("connect user");

        assert_eq!(device_index, 0);
        assert_eq!(user_index, 1);
        assert_eq!(manager.connections().len(), 2);
        assert!(
            manager
                .connections()
                .iter()
                .any(|connection| connection.client_type() == ClientType::Device)
        );
        assert!(
            manager
                .connections()
                .iter()
                .any(|connection| connection.client_type() == ClientType::User)
        );

        manager.close_all().await.expect("close manager");
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

        let (_db_dir, db) = create_test_database().await;
        db.set_control_plane_uri(&cp_uri)
            .await
            .expect("set control-plane uri");

        let mut manager = ClientManager::new(db).await.expect("create manager");
        let index = manager
            .connect(&registered.id, &STANDARD.encode(private_key))
            .await
            .expect("connect device");

        let connection = manager
            .connection_mut(index)
            .expect("connected client should exist");
        connection
            .send_text(&registered.id, "self-loopback-message")
            .await
            .expect("send message");

        let received = tokio::time::timeout(Duration::from_secs(2), connection.receive_message())
            .await
            .expect("timeout waiting for loopback message")
            .expect("receive message");
        assert_eq!(received.recipient, registered.id);
        assert_eq!(received.payload, b"self-loopback-message");

        manager.close_all().await.expect("close manager");
    })
    .await;
}

async fn create_test_database() -> (TempDir, ClientDatabase) {
    let dir = TempDir::new().expect("temp dir");
    let db_path = dir.path().join("client.sqlite3");
    let db = ClientDatabase::open(&db_path)
        .await
        .expect("open test client database");
    (dir, db)
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
