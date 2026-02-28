mod common;

use common::{
    ClientEntry, ClientType, ControlPlaneApi, ErrorResponse, LookupClientResponse,
    RegisterClientRequest, RegisterRelayRequest, RelayEntry, with_backend,
};
use hoshi_control_plane::{Config, ServerState};
use reqwest::Client;
use reqwest::StatusCode;
use tempfile::TempDir;

async fn register_client_created(api: &ControlPlaneApi, req: RegisterClientRequest) -> ClientEntry {
    let res = api.register_client(&req).await.unwrap();
    assert_eq!(res.status(), StatusCode::CREATED);
    res.json::<ClientEntry>().await.unwrap()
}

fn relay_request(guid: &str, public_key: &str, api_key: &str, port: u16) -> RegisterRelayRequest {
    RegisterRelayRequest {
        guid: guid.to_string(),
        public_key: public_key.to_string(),
        api_key: api_key.to_string(),
        port,
    }
}

#[test]
fn state_new_generates_and_persists_relay_api_key_when_missing() {
    let temp_dir = TempDir::new().expect("temp dir");
    let dir_root = temp_dir.path().to_str().expect("temp dir str");
    let mut config = Config::default()
        .set_dir_root(dir_root)
        .set_db_name("relay-key.sqlite3")
        .set_http_bind_addr("127.0.0.1:0")
        .expect("set_http_bind_addr");
    config.relay_api_key = None;

    let state = ServerState::new(config, std::time::Instant::now()).expect("state");
    let relay_api_key = state
        .config
        .relay_api_key
        .clone()
        .expect("relay_api_key should be generated");

    assert!(!relay_api_key.is_empty());

    let stored_api_key = state
        .db
        .get_relay_api_key()
        .expect("db get relay api key")
        .expect("db relay api key should exist");
    assert_eq!(relay_api_key, stored_api_key);
}

#[test]
fn state_new_uses_db_relay_api_key_when_config_is_missing() {
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
    let first_state = ServerState::new(first_config, std::time::Instant::now()).expect("state");
    drop(first_state);

    let mut second_config = Config::default()
        .set_dir_root(dir_root)
        .set_db_name(db_name)
        .set_http_bind_addr("127.0.0.1:0")
        .expect("set_http_bind_addr");
    second_config.relay_api_key = None;

    let second_state = ServerState::new(second_config, std::time::Instant::now()).expect("state");
    assert_eq!(
        second_state.config.relay_api_key.as_deref(),
        Some(configured_key)
    );
}

#[tokio::test]
async fn basic_http() {
    with_backend(|state| async move {
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
async fn basic_config_db_tests() {
    with_backend(|state| async move {
        let val = state.db.get_config("test").unwrap();
        assert!(val.is_none(), "'test' has a value before we set it");

        state.db.set_config("test", b"123").unwrap();
        let val = state.db.get_config("test").unwrap().unwrap();
        assert_eq!(val, b"123");

        state.db.set_config("test", b"").unwrap();
        let val = state.db.get_config("test").unwrap().unwrap();
        assert_eq!(val, b"");

        let mut vec: Vec<u8> = Vec::new();
        for i in 1..4096 {
            let v = i as u8;
            vec.push(v);
        }
        state.db.set_config("test", &vec).unwrap();
        let val = state.db.get_config("test").unwrap().unwrap();
        assert_eq!(val, vec);
    })
    .await;
}

#[tokio::test]
async fn register_client_success_returns_201_and_entry() {
    with_backend(|state| async move {
        let api = ControlPlaneApi::new(state.config.uri());
        let req = RegisterClientRequest {
            public_key: "AQID".to_string(),
            owner_id: None,
            client_type: ClientType::User,
        };

        let res = api.register_client(&req).await.unwrap();
        assert_eq!(res.status(), StatusCode::CREATED);
        let body = res.json::<ClientEntry>().await.unwrap();

        assert!(!body.id.is_empty());
        assert_eq!(body.owner_id, None);
        assert!(matches!(body.client_type, ClientType::User));
        assert_eq!(body.public_key, "AQID");
        assert!(body.created_at > 0);
        assert!(body.last_seen > 0);
    })
    .await;
}

#[tokio::test]
async fn register_client_duplicate_public_key_returns_409() {
    with_backend(|state| async move {
        let api = ControlPlaneApi::new(state.config.uri());
        let req = RegisterClientRequest {
            public_key: "AQID".to_string(),
            owner_id: None,
            client_type: ClientType::User,
        };

        let res = api.register_client(&req).await.unwrap();
        assert_eq!(res.status(), StatusCode::CREATED);

        let res = api.register_client(&req).await.unwrap();
        assert_eq!(res.status(), StatusCode::CONFLICT);
        let err = res.json::<ErrorResponse>().await.unwrap();
        assert_eq!(err.error, "client already exists");
    })
    .await;
}

#[tokio::test]
async fn register_client_invalid_base64_returns_400() {
    with_backend(|state| async move {
        let api = ControlPlaneApi::new(state.config.uri());
        let req = RegisterClientRequest {
            public_key: "not-base64@@".to_string(),
            owner_id: None,
            client_type: ClientType::User,
        };

        let res = api.register_client(&req).await.unwrap();
        assert_eq!(res.status(), StatusCode::BAD_REQUEST);
        let err = res.json::<ErrorResponse>().await.unwrap();
        assert_eq!(err.error, "invalid public_key base64");
    })
    .await;
}

#[tokio::test]
async fn lookup_client_returns_parent_and_direct_children_only() {
    with_backend(|state| async move {
        let api = ControlPlaneApi::new(state.config.uri());

        let parent = register_client_created(
            &api,
            RegisterClientRequest {
                public_key: "AQID".to_string(),
                owner_id: None,
                client_type: ClientType::User,
            },
        )
        .await;

        let child_one = register_client_created(
            &api,
            RegisterClientRequest {
                public_key: "BAUG".to_string(),
                owner_id: Some(parent.id.clone()),
                client_type: ClientType::Device,
            },
        )
        .await;

        let child_two = register_client_created(
            &api,
            RegisterClientRequest {
                public_key: "BwgJ".to_string(),
                owner_id: Some(parent.id.clone()),
                client_type: ClientType::Device,
            },
        )
        .await;

        let grandchild = register_client_created(
            &api,
            RegisterClientRequest {
                public_key: "CgsM".to_string(),
                owner_id: Some(child_one.id.clone()),
                client_type: ClientType::Device,
            },
        )
        .await;

        let res = api.lookup_client(&parent.id).await.unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let body = res.json::<LookupClientResponse>().await.unwrap();

        assert_eq!(body.client.id, parent.id);
        assert_eq!(body.children.len(), 2);
        let child_ids: Vec<String> = body.children.iter().map(|c| c.id.clone()).collect();
        assert!(child_ids.contains(&child_one.id));
        assert!(child_ids.contains(&child_two.id));
        assert!(!child_ids.contains(&grandchild.id));
    })
    .await;
}

#[tokio::test]
async fn lookup_client_missing_returns_404() {
    with_backend(|state| async move {
        let api = ControlPlaneApi::new(state.config.uri());
        let res = api.lookup_client("missing-guid").await.unwrap();
        assert_eq!(res.status(), StatusCode::NOT_FOUND);
        let err = res.json::<ErrorResponse>().await.unwrap();
        assert_eq!(err.error, "client not found");
    })
    .await;
}

#[tokio::test]
async fn register_relay_invalid_api_key_returns_401() {
    with_backend(|state| async move {
        let api = ControlPlaneApi::new(state.config.uri());
        let req = relay_request("11111111-1111-1111-1111-111111111111", "AQID", "1234", 4000);

        let res = api.register_relay(&req).await.unwrap();
        assert_eq!(res.status(), StatusCode::UNAUTHORIZED);
        let err = res.json::<ErrorResponse>().await.unwrap();
        assert_eq!(err.error, "invalid api key");
    })
    .await;
}

#[tokio::test]
async fn register_relay_invalid_guid_returns_400() {
    with_backend(|state| async move {
        let api = ControlPlaneApi::new(state.config.uri());
        let relay_api_key = state
            .config
            .relay_api_key
            .clone()
            .expect("relay_api_key should be set");
        let req = relay_request("not-a-guid", "AQID", &relay_api_key, 4000);

        let res = api.register_relay(&req).await.unwrap();
        assert_eq!(res.status(), StatusCode::BAD_REQUEST);
        let err = res.json::<ErrorResponse>().await.unwrap();
        assert_eq!(err.error, "invalid guid");
    })
    .await;
}

#[tokio::test]
async fn register_relay_invalid_public_key_returns_400() {
    with_backend(|state| async move {
        let api = ControlPlaneApi::new(state.config.uri());
        let relay_api_key = state
            .config
            .relay_api_key
            .clone()
            .expect("relay_api_key should be set");
        let req = relay_request(
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
    with_backend(|state| async move {
        let api = ControlPlaneApi::new(state.config.uri());
        let relay_api_key = state
            .config
            .relay_api_key
            .clone()
            .expect("relay_api_key should be set");
        let req = relay_request(
            "11111111-1111-1111-1111-111111111111",
            "AQID",
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
async fn list_relays_returns_flat_array_of_unique_entries() {
    with_backend(|state| async move {
        let api = ControlPlaneApi::new(state.config.uri());
        let relay_api_key = state
            .config
            .relay_api_key
            .clone()
            .expect("relay_api_key should be set");
        let guid_one = "11111111-1111-1111-1111-111111111111";
        let guid_two = "22222222-2222-2222-2222-222222222222";

        let res = api
            .register_relay(&relay_request(guid_two, "AQID", &relay_api_key, 4001))
            .await
            .unwrap();
        assert!(res.status().is_success());

        let res = api
            .register_relay(&relay_request(guid_one, "BAUG", &relay_api_key, 4002))
            .await
            .unwrap();
        assert!(res.status().is_success());

        let res = api
            .register_relay(&relay_request(guid_two, "BwgJ", &relay_api_key, 4010))
            .await
            .unwrap();
        assert!(res.status().is_success());

        let list_res = api.list_relays().await.unwrap();
        assert_eq!(list_res.status(), StatusCode::OK);

        let relays = list_res.json::<Vec<RelayEntry>>().await.unwrap();
        assert_eq!(relays.len(), 2);
        assert_eq!(relays[0].guid, guid_one);
        assert_eq!(relays[0].public_key, "BAUG");
        assert_eq!(relays[0].port, 4002);
        assert_eq!(relays[1].guid, guid_two);
        assert_eq!(relays[1].public_key, "BwgJ");
        assert_eq!(relays[1].port, 4010);
    })
    .await;
}
