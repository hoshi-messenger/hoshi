mod common;

use common::{with_control_plane, with_control_plane_and_relay, with_relay};
use hoshi_control_plane::api::NoisePublicKeyResponse;
use hoshi_relay::api::HealthzResponse;
use reqwest::StatusCode;

const NOISE_PATTERN: &str = "Noise_X_25519_ChaChaPoly_BLAKE2s";

#[tokio::test]
async fn with_control_plane_exposes_noise_key_endpoint() {
    with_control_plane(|state| async move {
        let client = reqwest::Client::new();
        let base_uri = state.config.uri();

        let response = client
            .get(format!("{}/noise/public-key", base_uri))
            .send()
            .await
            .expect("control-plane /noise/public-key response");

        assert_eq!(response.status(), StatusCode::OK);

        let body = response
            .json::<NoisePublicKeyResponse>()
            .await
            .expect("control-plane noise key payload");
        assert_eq!(body.pattern, NOISE_PATTERN);
        assert!(!body.public_key.is_empty());
    })
    .await;
}

#[tokio::test]
async fn with_relay_exposes_healthz_endpoint() {
    with_relay("http://127.0.0.1:1", "relay-api-key", |state| async move {
        let client = reqwest::Client::new();
        let base_uri = state.config.uri();

        let response = client
            .get(format!("{}/healthz", base_uri))
            .send()
            .await
            .expect("relay /healthz response");

        assert_eq!(response.status(), StatusCode::OK);

        let body = response
            .json::<HealthzResponse>()
            .await
            .expect("relay healthz payload");
        assert_eq!(body.status, "ok");
        assert_eq!(body.guid, state.config.guid);
        assert_eq!(body.control_plane_uri, "http://127.0.0.1:1");
    })
    .await;
}

#[tokio::test]
async fn with_control_plane_and_relay_runs_a_local_stack() {
    with_control_plane_and_relay(|control_plane, relay| async move {
        let client = reqwest::Client::new();
        let control_plane_uri = control_plane.config.uri();
        let relay_uri = relay.config.uri();

        let noise_response = client
            .get(format!("{}/noise/public-key", control_plane_uri))
            .send()
            .await
            .expect("control-plane probe");
        assert_eq!(noise_response.status(), StatusCode::OK);

        let health_response = client
            .get(format!("{}/healthz", relay_uri))
            .send()
            .await
            .expect("relay healthz");
        assert_eq!(health_response.status(), StatusCode::OK);

        let health = health_response
            .json::<HealthzResponse>()
            .await
            .expect("relay healthz payload");
        assert_eq!(health.status, "ok");
        assert_eq!(health.control_plane_uri, control_plane_uri);
    })
    .await;
}
