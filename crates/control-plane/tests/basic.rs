mod common;

use std::{future::Future, time::Duration};

use base64::{Engine as _, engine::general_purpose::STANDARD};
use common::{
    ClientEntry, ClientType, ControlPlaneApi, ErrorResponse, IssueRelayTokenRequest,
    IssueRelayTokenResponse, LookupClientResponse, NoisePublicKeyResponse, RegisterClientRequest,
    RegisterRelayRequest, RelayEntry, RelayJwtPublicKeyResponse, with_control_plane,
};
use hoshi_control_plane::{Config, RelayPresence, ServerState};
use hoshi_protocol::{
    control_plane::{
        ClientRegistrationProofPayload, RelayRegistrationProofPayload, RelayTokenProofPayload,
    },
    noise::{
        REGISTRATION_NOISE_PATTERN, create_registration_handshake, derive_public_key,
        encode_base64, generate_static_private_key,
    },
};
use jsonwebtoken::{Algorithm, DecodingKey, Validation, decode};
use reqwest::Client;
use reqwest::StatusCode;
use serde::Deserialize;
use tempfile::TempDir;

#[derive(Debug, Deserialize)]
struct RelayTokenClaims {
    sub: String,
    exp: i64,
    iat: i64,
    client_type: ClientType,
}

fn generate_noise_keypair() -> (String, [u8; 32]) {
    let private_key = generate_static_private_key().expect("generate noise keypair");
    let public_key = encode_base64(&derive_public_key(&private_key));
    (public_key, private_key)
}

fn create_noise_handshake(
    client_private_key: &[u8; 32],
    server_public_key_b64: &str,
    proof_payload: &[u8],
) -> String {
    let handshake =
        create_registration_handshake(client_private_key, server_public_key_b64, proof_payload)
            .expect("write noise handshake");
    STANDARD.encode(handshake)
}

async fn fetch_noise_public_key(api: &ControlPlaneApi) -> NoisePublicKeyResponse {
    let res = api.get_noise_public_key().await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    res.json::<NoisePublicKeyResponse>().await.unwrap()
}

async fn client_request(
    api: &ControlPlaneApi,
    public_key: &str,
    private_key: &[u8; 32],
    client_type: ClientType,
) -> RegisterClientRequest {
    let server_noise = fetch_noise_public_key(api).await;
    let proof_payload = serde_json::to_vec(&ClientRegistrationProofPayload {
        public_key: public_key.to_string(),
        client_type: client_type.clone(),
    })
    .expect("serialize client proof payload");
    let noise_handshake =
        create_noise_handshake(private_key, &server_noise.public_key, &proof_payload);

    RegisterClientRequest {
        public_key: public_key.to_string(),
        client_type,
        noise_handshake,
    }
}

async fn relay_request(
    api: &ControlPlaneApi,
    guid: &str,
    public_key: &str,
    private_key: &[u8; 32],
    api_key: &str,
    port: u16,
) -> RegisterRelayRequest {
    let server_noise = fetch_noise_public_key(api).await;
    let proof_payload = serde_json::to_vec(&RelayRegistrationProofPayload {
        public_key: public_key.to_string(),
        guid: guid.to_string(),
        api_key: api_key.to_string(),
        port,
    })
    .expect("serialize relay proof payload");
    let noise_handshake =
        create_noise_handshake(private_key, &server_noise.public_key, &proof_payload);

    RegisterRelayRequest {
        guid: guid.to_string(),
        public_key: public_key.to_string(),
        api_key: api_key.to_string(),
        port,
        noise_handshake,
    }
}

async fn relay_token_request(
    api: &ControlPlaneApi,
    public_key: &str,
    private_key: &[u8; 32],
) -> IssueRelayTokenRequest {
    let server_noise = fetch_noise_public_key(api).await;
    let proof_payload = serde_json::to_vec(&RelayTokenProofPayload {
        public_key: public_key.to_string(),
    })
    .expect("serialize token proof");
    let noise_handshake =
        create_noise_handshake(private_key, &server_noise.public_key, &proof_payload);

    IssueRelayTokenRequest {
        public_key: public_key.to_string(),
        noise_handshake,
    }
}

fn relay_request_without_valid_proof(
    guid: &str,
    public_key: &str,
    api_key: &str,
    port: u16,
) -> RegisterRelayRequest {
    RegisterRelayRequest {
        guid: guid.to_string(),
        public_key: public_key.to_string(),
        api_key: api_key.to_string(),
        port,
        noise_handshake: "AQID".to_string(),
    }
}

async fn register_client_created(api: &ControlPlaneApi, req: RegisterClientRequest) -> ClientEntry {
    let res = api.register_client(&req).await.unwrap();
    assert_eq!(res.status(), StatusCode::CREATED);
    res.json::<ClientEntry>().await.unwrap()
}

async fn with_control_plane_api<F, Fut>(test: F)
where
    F: FnOnce(ServerState, ControlPlaneApi) -> Fut,
    Fut: Future<Output = ()>,
{
    with_control_plane(|state| async move {
        let api = ControlPlaneApi::new(state.config.uri());
        test(state, api).await;
    })
    .await;
}

#[tokio::test]
async fn state_new_generates_and_persists_relay_api_key_when_missing() {
    let temp_dir = TempDir::new().expect("temp dir");
    let dir_root = temp_dir.path().to_str().expect("temp dir str");
    let mut config = Config::default()
        .set_dir_root(dir_root)
        .set_db_name("relay-key.sqlite3")
        .set_http_bind_addr("127.0.0.1:0")
        .expect("set_http_bind_addr");
    config.relay_api_key = None;

    let state = ServerState::new(config, std::time::Instant::now())
        .await
        .expect("state");
    let relay_api_key = state
        .config
        .relay_api_key
        .clone()
        .expect("relay_api_key should be generated");

    assert!(!relay_api_key.is_empty());

    let stored_api_key = state
        .db
        .get_relay_api_key()
        .await
        .expect("db get relay api key")
        .expect("db relay api key should exist");
    assert_eq!(relay_api_key, stored_api_key);
}

#[tokio::test]
async fn state_new_uses_db_relay_api_key_when_config_is_missing() {
    let temp_dir = TempDir::new().expect("temp dir");
    let dir_root = temp_dir.path().to_str().expect("temp dir str");
    let db_name = "relay-key.sqlite3";

    let configured_key = "persisted-relay-api-key";
    let first_config = Config::default()
        .set_dir_root(dir_root)
        .set_db_name(db_name)
        .set_relay_api_key(configured_key)
        .set_http_bind_addr("127.0.0.1:0")
        .expect("set_http_bind_addr");
    let first_state = ServerState::new(first_config, std::time::Instant::now())
        .await
        .expect("state");
    drop(first_state);

    let mut second_config = Config::default()
        .set_dir_root(dir_root)
        .set_db_name(db_name)
        .set_http_bind_addr("127.0.0.1:0")
        .expect("set_http_bind_addr");
    second_config.relay_api_key = None;

    let second_state = ServerState::new(second_config, std::time::Instant::now())
        .await
        .expect("state");
    assert_eq!(
        second_state.config.relay_api_key.as_deref(),
        Some(configured_key)
    );
}

#[tokio::test]
async fn state_new_generates_and_persists_noise_static_private_key_when_missing() {
    let temp_dir = TempDir::new().expect("temp dir");
    let dir_root = temp_dir.path().to_str().expect("temp dir str");
    let mut config = Config::default()
        .set_dir_root(dir_root)
        .set_db_name("noise-key.sqlite3")
        .set_http_bind_addr("127.0.0.1:0")
        .expect("set_http_bind_addr");
    config.noise_static_private_key = None;

    let state = ServerState::new(config, std::time::Instant::now())
        .await
        .expect("state");
    let noise_key = state
        .config
        .noise_static_private_key
        .clone()
        .expect("noise key should be generated");

    let decoded = STANDARD
        .decode(&noise_key)
        .expect("noise key should be base64");
    assert_eq!(decoded.len(), 32);

    let stored_noise_key = state
        .db
        .get_noise_static_private_key()
        .await
        .expect("db get noise key")
        .expect("db noise key should exist");
    assert_eq!(noise_key, stored_noise_key);
}

#[tokio::test]
async fn state_new_uses_db_noise_static_private_key_when_config_is_missing() {
    let temp_dir = TempDir::new().expect("temp dir");
    let dir_root = temp_dir.path().to_str().expect("temp dir str");
    let db_name = "noise-key.sqlite3";

    let configured_key = STANDARD.encode([7_u8; 32]);
    let first_config = Config::default()
        .set_dir_root(dir_root)
        .set_db_name(db_name)
        .set_noise_static_private_key(&configured_key)
        .set_http_bind_addr("127.0.0.1:0")
        .expect("set_http_bind_addr");
    let first_state = ServerState::new(first_config, std::time::Instant::now())
        .await
        .expect("state");
    drop(first_state);

    let mut second_config = Config::default()
        .set_dir_root(dir_root)
        .set_db_name(db_name)
        .set_http_bind_addr("127.0.0.1:0")
        .expect("set_http_bind_addr");
    second_config.noise_static_private_key = None;

    let second_state = ServerState::new(second_config, std::time::Instant::now())
        .await
        .expect("state");
    assert_eq!(
        second_state.config.noise_static_private_key.as_deref(),
        Some(configured_key.as_str())
    );
}

#[tokio::test]
async fn basic_http() {
    with_control_plane(|state| async move {
        let client = Client::builder()
            .build()
            .expect("Failed to create reqwest client");

        let base = state.config.uri();
        let res = client.get(format!("{}/", base)).send().await.unwrap();
        let text = res.text().await.unwrap();

        assert!(
            text.contains("Hoshi"),
            "Hoshi doesn't appear on landing page"
        );
    })
    .await;
}

#[tokio::test]
async fn noise_public_key_endpoint_returns_pattern_and_valid_key() {
    with_control_plane_api(|_state, api| async move {
        let res = api.get_noise_public_key().await.unwrap();
        assert_eq!(res.status(), StatusCode::OK);

        let body = res.json::<NoisePublicKeyResponse>().await.unwrap();
        assert_eq!(body.pattern, REGISTRATION_NOISE_PATTERN);
        let key = STANDARD.decode(body.public_key).unwrap();
        assert_eq!(key.len(), 32);
    })
    .await;
}

#[tokio::test]
async fn basic_config_db_tests() {
    with_control_plane(|state| async move {
        let val = state.db.get_config("test").await.unwrap();
        assert!(val.is_none(), "'test' has a value before we set it");

        state.db.set_config("test", b"123").await.unwrap();
        let val = state.db.get_config("test").await.unwrap().unwrap();
        assert_eq!(val, b"123");

        state.db.set_config("test", b"").await.unwrap();
        let val = state.db.get_config("test").await.unwrap().unwrap();
        assert_eq!(val, b"");

        let mut vec: Vec<u8> = Vec::new();
        for i in 1..4096 {
            let v = i as u8;
            vec.push(v);
        }
        state.db.set_config("test", &vec).await.unwrap();
        let val = state.db.get_config("test").await.unwrap().unwrap();
        assert_eq!(val, vec);
    })
    .await;
}

#[tokio::test]
async fn register_client_success_returns_201_and_entry() {
    with_control_plane_api(|_state, api| async move {
        let (public_key, private_key) = generate_noise_keypair();
        let req = client_request(&api, &public_key, &private_key, ClientType::User).await;

        let res = api.register_client(&req).await.unwrap();
        assert_eq!(res.status(), StatusCode::CREATED);
        let body = res.json::<ClientEntry>().await.unwrap();

        assert!(!body.id.is_empty());
        assert!(matches!(body.client_type, ClientType::User));
        assert_eq!(body.public_key, public_key);
    })
    .await;
}

#[tokio::test]
async fn register_client_duplicate_public_key_returns_existing_entry() {
    with_control_plane_api(|_state, api| async move {
        let (public_key, private_key) = generate_noise_keypair();
        let req = client_request(&api, &public_key, &private_key, ClientType::User).await;

        let res = api.register_client(&req).await.unwrap();
        assert_eq!(res.status(), StatusCode::CREATED);
        let first = res.json::<ClientEntry>().await.unwrap();

        let res = api.register_client(&req).await.unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let second = res.json::<ClientEntry>().await.unwrap();
        assert_eq!(second.id, first.id);
        assert_eq!(second.public_key, first.public_key);
        assert_eq!(second.client_type, first.client_type);
    })
    .await;
}

#[tokio::test]
async fn register_client_invalid_base64_returns_400() {
    with_control_plane_api(|_state, api| async move {
        let req = RegisterClientRequest {
            public_key: "not-base64@@".to_string(),
            client_type: ClientType::User,
            noise_handshake: "AQID".to_string(),
        };

        let res = api.register_client(&req).await.unwrap();
        assert_eq!(res.status(), StatusCode::BAD_REQUEST);
        let err = res.json::<ErrorResponse>().await.unwrap();
        assert_eq!(err.error, "invalid public_key base64");
    })
    .await;
}

#[tokio::test]
async fn register_client_invalid_noise_handshake_base64_returns_400() {
    with_control_plane_api(|_state, api| async move {
        let (public_key, _) = generate_noise_keypair();
        let req = RegisterClientRequest {
            public_key,
            client_type: ClientType::User,
            noise_handshake: "not-base64@@".to_string(),
        };

        let res = api.register_client(&req).await.unwrap();
        assert_eq!(res.status(), StatusCode::BAD_REQUEST);
        let err = res.json::<ErrorResponse>().await.unwrap();
        assert_eq!(err.error, "invalid noise_handshake base64");
    })
    .await;
}

#[tokio::test]
async fn register_client_invalid_registration_proof_returns_400() {
    with_control_plane_api(|_state, api| async move {
        let (public_key, _) = generate_noise_keypair();
        let req = RegisterClientRequest {
            public_key,
            client_type: ClientType::User,
            noise_handshake: "AQID".to_string(),
        };

        let res = api.register_client(&req).await.unwrap();
        assert_eq!(res.status(), StatusCode::BAD_REQUEST);
        let err = res.json::<ErrorResponse>().await.unwrap();
        assert_eq!(err.error, "invalid registration proof");
    })
    .await;
}

#[tokio::test]
async fn register_client_rejects_proof_when_payload_is_tampered() {
    with_control_plane_api(|_state, api| async move {
        let (public_key, private_key) = generate_noise_keypair();
        let mut req = client_request(&api, &public_key, &private_key, ClientType::User).await;
        req.client_type = ClientType::Device;

        let res = api.register_client(&req).await.unwrap();
        assert_eq!(res.status(), StatusCode::BAD_REQUEST);
        let err = res.json::<ErrorResponse>().await.unwrap();
        assert_eq!(err.error, "invalid registration proof");
    })
    .await;
}

#[tokio::test]
async fn register_client_rejects_proof_when_signer_key_does_not_match_claimed_public_key() {
    with_control_plane_api(|_state, api| async move {
        let (claimed_public_key, _) = generate_noise_keypair();
        let (_, wrong_private_key) = generate_noise_keypair();
        let req = client_request(
            &api,
            &claimed_public_key,
            &wrong_private_key,
            ClientType::User,
        )
        .await;

        let res = api.register_client(&req).await.unwrap();
        assert_eq!(res.status(), StatusCode::BAD_REQUEST);
        let err = res.json::<ErrorResponse>().await.unwrap();
        assert_eq!(err.error, "invalid registration proof");
    })
    .await;
}

#[tokio::test]
async fn lookup_client_returns_public_key() {
    with_control_plane_api(|_state, api| async move {
        let (parent_key, parent_private) = generate_noise_keypair();
        let parent = register_client_created(
            &api,
            client_request(&api, &parent_key, &parent_private, ClientType::User).await,
        )
        .await;

        let res = api.lookup_client(&parent.id).await.unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let body = res.json::<LookupClientResponse>().await.unwrap();
        assert_eq!(body.public_key, parent.public_key);
    })
    .await;
}

#[tokio::test]
async fn lookup_client_missing_returns_404() {
    with_control_plane_api(|_state, api| async move {
        let res = api.lookup_client("missing-guid").await.unwrap();
        assert_eq!(res.status(), StatusCode::NOT_FOUND);
        let err = res.json::<ErrorResponse>().await.unwrap();
        assert_eq!(err.error, "client not found");
    })
    .await;
}

#[tokio::test]
async fn register_relay_invalid_api_key_returns_401() {
    with_control_plane_api(|_state, api| async move {
        let (public_key, _) = generate_noise_keypair();
        let req = relay_request_without_valid_proof(
            "11111111-1111-1111-1111-111111111111",
            &public_key,
            "1234",
            4000,
        );

        let res = api.register_relay(&req).await.unwrap();
        assert_eq!(res.status(), StatusCode::UNAUTHORIZED);
        let err = res.json::<ErrorResponse>().await.unwrap();
        assert_eq!(err.error, "invalid api key");
    })
    .await;
}

#[tokio::test]
async fn register_relay_invalid_guid_returns_400() {
    with_control_plane_api(|state, api| async move {
        let relay_api_key = state
            .config
            .relay_api_key
            .clone()
            .expect("relay_api_key should be set");
        let (public_key, _) = generate_noise_keypair();
        let req =
            relay_request_without_valid_proof("not-a-guid", &public_key, &relay_api_key, 4000);

        let res = api.register_relay(&req).await.unwrap();
        assert_eq!(res.status(), StatusCode::BAD_REQUEST);
        let err = res.json::<ErrorResponse>().await.unwrap();
        assert_eq!(err.error, "invalid guid");
    })
    .await;
}

#[tokio::test]
async fn register_relay_invalid_public_key_returns_400() {
    with_control_plane_api(|state, api| async move {
        let relay_api_key = state
            .config
            .relay_api_key
            .clone()
            .expect("relay_api_key should be set");
        let req = relay_request_without_valid_proof(
            "11111111-1111-1111-1111-111111111111",
            "not-base64@@",
            &relay_api_key,
            4000,
        );

        let res = api.register_relay(&req).await.unwrap();
        assert_eq!(res.status(), StatusCode::BAD_REQUEST);
        let err = res.json::<ErrorResponse>().await.unwrap();
        assert_eq!(err.error, "invalid public_key base64");
    })
    .await;
}

#[tokio::test]
async fn register_relay_invalid_port_returns_400() {
    with_control_plane_api(|state, api| async move {
        let relay_api_key = state
            .config
            .relay_api_key
            .clone()
            .expect("relay_api_key should be set");
        let (public_key, _) = generate_noise_keypair();
        let req = relay_request_without_valid_proof(
            "11111111-1111-1111-1111-111111111111",
            &public_key,
            &relay_api_key,
            0,
        );

        let res = api.register_relay(&req).await.unwrap();
        assert_eq!(res.status(), StatusCode::BAD_REQUEST);
        let err = res.json::<ErrorResponse>().await.unwrap();
        assert_eq!(err.error, "invalid port");
    })
    .await;
}

#[tokio::test]
async fn register_relay_invalid_registration_proof_returns_400() {
    with_control_plane_api(|state, api| async move {
        let relay_api_key = state
            .config
            .relay_api_key
            .clone()
            .expect("relay_api_key should be set");
        let (public_key, _) = generate_noise_keypair();
        let req = relay_request_without_valid_proof(
            "11111111-1111-1111-1111-111111111111",
            &public_key,
            &relay_api_key,
            4000,
        );

        let res = api.register_relay(&req).await.unwrap();
        assert_eq!(res.status(), StatusCode::BAD_REQUEST);
        let err = res.json::<ErrorResponse>().await.unwrap();
        assert_eq!(err.error, "invalid registration proof");
    })
    .await;
}

#[tokio::test]
async fn register_relay_invalid_noise_handshake_base64_returns_400() {
    with_control_plane_api(|state, api| async move {
        let relay_api_key = state
            .config
            .relay_api_key
            .clone()
            .expect("relay_api_key should be set");
        let (public_key, _) = generate_noise_keypair();
        let mut req = relay_request_without_valid_proof(
            "11111111-1111-1111-1111-111111111111",
            &public_key,
            &relay_api_key,
            4000,
        );
        req.noise_handshake = "not-base64@@".to_string();

        let res = api.register_relay(&req).await.unwrap();
        assert_eq!(res.status(), StatusCode::BAD_REQUEST);
        let err = res.json::<ErrorResponse>().await.unwrap();
        assert_eq!(err.error, "invalid noise_handshake base64");
    })
    .await;
}

#[tokio::test]
async fn register_relay_rejects_proof_when_payload_is_tampered() {
    with_control_plane_api(|state, api| async move {
        let relay_api_key = state
            .config
            .relay_api_key
            .clone()
            .expect("relay_api_key should be set");
        let (public_key, private_key) = generate_noise_keypair();
        let mut req = relay_request(
            &api,
            "11111111-1111-1111-1111-111111111111",
            &public_key,
            &private_key,
            &relay_api_key,
            4000,
        )
        .await;
        req.port = 4001;

        let res = api.register_relay(&req).await.unwrap();
        assert_eq!(res.status(), StatusCode::BAD_REQUEST);
        let err = res.json::<ErrorResponse>().await.unwrap();
        assert_eq!(err.error, "invalid registration proof");
    })
    .await;
}

#[tokio::test]
async fn register_relay_rejects_proof_when_signer_key_does_not_match_claimed_public_key() {
    with_control_plane_api(|state, api| async move {
        let relay_api_key = state
            .config
            .relay_api_key
            .clone()
            .expect("relay_api_key should be set");
        let (claimed_public_key, _) = generate_noise_keypair();
        let (_, wrong_private_key) = generate_noise_keypair();
        let req = relay_request(
            &api,
            "11111111-1111-1111-1111-111111111111",
            &claimed_public_key,
            &wrong_private_key,
            &relay_api_key,
            4000,
        )
        .await;

        let res = api.register_relay(&req).await.unwrap();
        assert_eq!(res.status(), StatusCode::BAD_REQUEST);
        let err = res.json::<ErrorResponse>().await.unwrap();
        assert_eq!(err.error, "invalid registration proof");
    })
    .await;
}

#[tokio::test]
async fn list_relays_returns_flat_array_of_unique_entries() {
    with_control_plane_api(|state, api| async move {
        let relay_api_key = state
            .config
            .relay_api_key
            .clone()
            .expect("relay_api_key should be set");
        let guid_one = "11111111-1111-1111-1111-111111111111";
        let guid_two = "22222222-2222-2222-2222-222222222222";

        let (public_two_v1, private_two_v1) = generate_noise_keypair();
        let res = api
            .register_relay(
                &relay_request(
                    &api,
                    guid_two,
                    &public_two_v1,
                    &private_two_v1,
                    &relay_api_key,
                    4001,
                )
                .await,
            )
            .await
            .unwrap();
        assert!(res.status().is_success());

        let (public_one, private_one) = generate_noise_keypair();
        let res = api
            .register_relay(
                &relay_request(
                    &api,
                    guid_one,
                    &public_one,
                    &private_one,
                    &relay_api_key,
                    4002,
                )
                .await,
            )
            .await
            .unwrap();
        assert!(res.status().is_success());

        let (public_two_v2, private_two_v2) = generate_noise_keypair();
        let res = api
            .register_relay(
                &relay_request(
                    &api,
                    guid_two,
                    &public_two_v2,
                    &private_two_v2,
                    &relay_api_key,
                    4010,
                )
                .await,
            )
            .await
            .unwrap();
        assert!(res.status().is_success());

        let list_res = api.list_relays().await.unwrap();
        assert_eq!(list_res.status(), StatusCode::OK);

        let relays = list_res.json::<Vec<RelayEntry>>().await.unwrap();
        assert_eq!(relays.len(), 2);
        assert_eq!(relays[0].guid, guid_one);
        assert_eq!(relays[0].public_key, public_one);
        assert_eq!(relays[0].port, 4002);
        assert_eq!(relays[1].guid, guid_two);
        assert_eq!(relays[1].public_key, public_two_v2);
        assert_eq!(relays[1].port, 4010);
    })
    .await;
}

#[tokio::test]
async fn relay_jwt_public_key_endpoint_exposes_eddsa_key() {
    with_control_plane_api(|_state, api| async move {
        let res = api.get_relay_jwt_public_key().await.unwrap();
        assert_eq!(res.status(), StatusCode::OK);

        let body = res.json::<RelayJwtPublicKeyResponse>().await.unwrap();
        assert_eq!(body.alg, "EdDSA");
        assert!(!body.x.is_empty());
    })
    .await;
}

#[tokio::test]
async fn issue_relay_token_returns_signed_claims_for_single_identity() {
    with_control_plane_api(|_state, api| async move {
        let (device_public, device_private) = generate_noise_keypair();
        let device = register_client_created(
            &api,
            client_request(&api, &device_public, &device_private, ClientType::Device).await,
        )
        .await;

        let token_res = api
            .issue_relay_token(&relay_token_request(&api, &device_public, &device_private).await)
            .await
            .unwrap();
        assert_eq!(token_res.status(), StatusCode::OK);
        let token_body = token_res.json::<IssueRelayTokenResponse>().await.unwrap();
        assert_eq!(token_body.guid, device.id);
        assert_eq!(token_body.client_type, ClientType::Device);

        let key_res = api.get_relay_jwt_public_key().await.unwrap();
        assert_eq!(key_res.status(), StatusCode::OK);
        let key = key_res.json::<RelayJwtPublicKeyResponse>().await.unwrap();

        let token = decode::<RelayTokenClaims>(
            &token_body.token,
            &DecodingKey::from_ed_components(&key.x).expect("valid ed key"),
            &Validation::new(Algorithm::EdDSA),
        )
        .expect("decode relay token");
        assert_eq!(token.claims.sub, device.id);
        assert_eq!(token.claims.client_type, ClientType::Device);
        assert!(token.claims.exp > token.claims.iat);
    })
    .await;
}

#[tokio::test]
async fn issue_relay_token_updates_client_last_seen() {
    with_control_plane_api(|state, api| async move {
        let (device_public, device_private) = generate_noise_keypair();
        let device = register_client_created(
            &api,
            client_request(&api, &device_public, &device_private, ClientType::Device).await,
        )
        .await;

        let before = state
            .db
            .get_client(&device.id)
            .await
            .expect("lookup registered client before token")
            .expect("registered client exists")
            .last_seen;

        tokio::time::sleep(Duration::from_secs(1)).await;

        let token_res = api
            .issue_relay_token(&relay_token_request(&api, &device_public, &device_private).await)
            .await
            .unwrap();
        assert_eq!(token_res.status(), StatusCode::OK);

        let after = state
            .db
            .get_client(&device.id)
            .await
            .expect("lookup registered client after token")
            .expect("registered client exists")
            .last_seen;

        assert!(
            after > before,
            "expected last_seen to increase, before={before}, after={after}"
        );
    })
    .await;
}

#[tokio::test]
async fn list_relays_filters_stale_entries() {
    with_control_plane_api(|state, api| async move {
        let relay_api_key = state
            .config
            .relay_api_key
            .clone()
            .expect("relay_api_key should be set");

        let live_guid = "44444444-4444-4444-4444-444444444444";
        let stale_guid = "55555555-5555-5555-5555-555555555555";

        let (live_public, live_private) = generate_noise_keypair();
        let res = api
            .register_relay(
                &relay_request(
                    &api,
                    live_guid,
                    &live_public,
                    &live_private,
                    &relay_api_key,
                    4700,
                )
                .await,
            )
            .await
            .unwrap();
        assert!(res.status().is_success());

        state.relays.insert(
            stale_guid.to_string(),
            RelayPresence {
                entry: RelayEntry {
                    guid: stale_guid.to_string(),
                    public_key: "stale-key".to_string(),
                    ip: "127.0.0.1".to_string(),
                    port: 4701,
                },
                last_seen: 0,
            },
        );

        let list_res = api.list_relays().await.unwrap();
        assert_eq!(list_res.status(), StatusCode::OK);

        let relays = list_res.json::<Vec<RelayEntry>>().await.unwrap();
        assert_eq!(relays.len(), 1);
        assert_eq!(relays[0].guid, live_guid);
        assert!(
            state.relays.get(stale_guid).is_none(),
            "stale relay should be pruned after listing"
        );
    })
    .await;
}
