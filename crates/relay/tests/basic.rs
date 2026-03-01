mod common;

use std::time::Duration;

use base64::{Engine as _, engine::general_purpose::STANDARD};
use common::{HealthzResponse, RelayApi, with_backend, write_test_config};
use hoshi_relay::{Config, ServerState, create_listeners, run};
use reqwest::StatusCode;
use tempfile::TempDir;
use uuid::Uuid;

#[tokio::test]
async fn config_load_creates_file_and_errors_without_api_key() {
    let dir = TempDir::new().expect("temp dir");
    let config_path = dir.path().join("relay.toml");

    let err = Config::load_from_path(&config_path).expect_err("config should require api_key");
    assert!(
        err.to_string().contains("api_key"),
        "expected api_key error, got: {err:#}"
    );

    let raw = std::fs::read_to_string(&config_path).expect("config file should exist");
    let value: toml::Value = toml::from_str(&raw).expect("valid toml");

    let guid = value
        .get("guid")
        .and_then(toml::Value::as_str)
        .expect("guid should be present");
    Uuid::parse_str(guid).expect("guid should be valid");

    let noise_key = value
        .get("noise_static_private_key")
        .and_then(toml::Value::as_str)
        .expect("noise key should be present");
    let decoded = STANDARD
        .decode(noise_key)
        .expect("noise key should be base64");
    assert_eq!(decoded.len(), 32);

    let api_key = value
        .get("api_key")
        .and_then(toml::Value::as_str)
        .expect("api_key should be present");
    assert_eq!(api_key, "");
}

#[tokio::test]
async fn config_load_generates_missing_identity_and_persists() {
    let dir = TempDir::new().expect("temp dir");
    let config_path = dir.path().join("relay.toml");
    write_test_config(
        &config_path,
        "relay-api-key",
        "127.0.0.1:2700",
        "http://127.0.0.1:2600",
    )
    .expect("write config");

    let config = Config::load_from_path(&config_path).expect("load config");
    Uuid::parse_str(&config.guid).expect("guid should be valid");
    let decoded = STANDARD
        .decode(&config.noise_static_private_key)
        .expect("noise key should be base64");
    assert_eq!(decoded.len(), 32);
    assert_eq!(config.api_key, "relay-api-key");

    let raw = std::fs::read_to_string(&config_path).expect("config file");
    let value: toml::Value = toml::from_str(&raw).expect("valid toml");
    assert_eq!(
        value
            .get("guid")
            .and_then(toml::Value::as_str)
            .expect("persisted guid"),
        config.guid
    );
    assert_eq!(
        value
            .get("noise_static_private_key")
            .and_then(toml::Value::as_str)
            .expect("persisted key"),
        config.noise_static_private_key
    );
}

#[tokio::test]
async fn config_load_rejects_invalid_noise_key() {
    let dir = TempDir::new().expect("temp dir");
    let config_path = dir.path().join("relay.toml");
    let config = format!(
        r#"
http_bind_address = "127.0.0.1:2700"
guid = "{}"
noise_static_private_key = "AQID"
api_key = "relay-api-key"
"#,
        Uuid::now_v7()
    );
    std::fs::write(&config_path, config).expect("write config");

    let err = Config::load_from_path(&config_path).expect_err("invalid key should fail");
    assert!(
        err.to_string().contains("noise_static_private_key"),
        "expected noise key error, got: {err:#}"
    );
}

#[tokio::test]
async fn config_load_rejects_blank_api_key() {
    let dir = TempDir::new().expect("temp dir");
    let config_path = dir.path().join("relay.toml");
    let noise_key = STANDARD.encode([7_u8; 32]);
    let config = format!(
        r#"
http_bind_address = "127.0.0.1:2700"
guid = "{}"
noise_static_private_key = "{noise_key}"
api_key = "   "
"#,
        Uuid::now_v7()
    );
    std::fs::write(&config_path, config).expect("write config");

    let err = Config::load_from_path(&config_path).expect_err("blank api_key should fail");
    assert!(
        err.to_string().contains("api_key"),
        "expected api_key error, got: {err:#}"
    );
}

#[tokio::test]
async fn basic_http_routes() {
    with_backend(|state| async move {
        let api = RelayApi::new(state.config.uri());

        let index = api.get_index().await.expect("index response");
        assert_eq!(index.status(), StatusCode::OK);
        let index_text = index.text().await.expect("index text");
        assert!(
            index_text.contains("Hoshi relay"),
            "landing page should contain Hoshi relay"
        );

        let healthz = api.get_healthz().await.expect("healthz response");
        assert_eq!(healthz.status(), StatusCode::OK);
        let body = healthz
            .json::<HealthzResponse>()
            .await
            .expect("healthz json body");
        assert_eq!(body.status, "ok");
        assert_eq!(body.guid, state.config.guid);
        assert_eq!(body.control_plane_uri, state.config.control_plane_uri);
    })
    .await;
}

#[tokio::test]
async fn startup_continues_when_control_plane_is_unreachable() {
    let process_start = std::time::Instant::now();
    let dir = TempDir::new().expect("temp dir");
    let config_path = dir.path().join("relay.toml");
    write_test_config(
        &config_path,
        "relay-api-key",
        "127.0.0.1:0",
        "http://127.0.0.1:1",
    )
    .expect("write config");

    let config = Config::load_from_path(&config_path).expect("load config");
    let (http_listener, http_addr) = create_listeners(&config).expect("create listeners");
    let config = config.update_bound_addresses(http_addr);
    let state = ServerState::new(config, process_start)
        .await
        .expect("create relay state");
    let base_uri = state.config.uri();

    let kill = async move {
        let client = reqwest::Client::new();
        let mut healthy = false;
        for _ in 0..40 {
            match client.get(format!("{}/healthz", base_uri)).send().await {
                Ok(response) if response.status() == StatusCode::OK => {
                    healthy = true;
                    break;
                }
                _ => tokio::time::sleep(Duration::from_millis(50)).await,
            }
        }

        assert!(healthy, "relay should serve http even when cp probe fails");
    };

    run(state, http_listener, kill).await;
}
