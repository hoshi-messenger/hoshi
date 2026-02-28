mod common;

use common::{
    ClientEntry, ClientType, ControlPlaneApi, ErrorResponse, LookupClientResponse,
    RegisterClientRequest, RegisterRelayRequest, with_backend,
};
use reqwest::Client;
use reqwest::StatusCode;

async fn register_client_created(api: &ControlPlaneApi, req: RegisterClientRequest) -> ClientEntry {
    let res = api.register_client(&req).await.unwrap();
    assert_eq!(res.status(), StatusCode::CREATED);
    res.json::<ClientEntry>().await.unwrap()
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
async fn register_relay_requires_api_key() {
    with_backend(|state| async move {
        let api = ControlPlaneApi::new(state.config.uri());
        let req = RegisterRelayRequest {
            public_key: "AQID".to_string(),
            guid: None,
        };

        let res = api.register_relay(None, &req).await.unwrap();
        assert_eq!(res.status(), StatusCode::UNAUTHORIZED);
        let err = res.json::<ErrorResponse>().await.unwrap();
        assert_eq!(err.error, "missing x-api-key");
    })
    .await;
}

#[tokio::test]
async fn register_relay_placeholder_returns_501() {
    with_backend(|state| async move {
        let api = ControlPlaneApi::new(state.config.uri());
        let req = RegisterRelayRequest {
            public_key: "AQID".to_string(),
            guid: None,
        };

        let res = api
            .register_relay(Some("relay-secret"), &req)
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::NOT_IMPLEMENTED);
        let err = res.json::<ErrorResponse>().await.unwrap();
        assert_eq!(err.error, "relay registration not implemented yet");
    })
    .await;
}
