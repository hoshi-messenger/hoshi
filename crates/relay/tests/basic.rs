mod common;

use common::{RelayApi, with_relay};
use futures::{SinkExt, StreamExt};
use hoshi_clientlib::{HoshiEnvelope, identity::HoshiIdentity};
use hoshi_relay::api;
use reqwest::StatusCode;
use reqwest::header::{CONNECTION, SEC_WEBSOCKET_VERSION, UPGRADE, USER_AGENT};
use reqwest_websocket::{Message, RequestBuilderExt};
use tokio::time::{Duration, sleep, timeout};

#[tokio::test]
async fn basic_http_routes() {
    with_relay(|state| async move {
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
            .json::<api::RelayStatusResponse>()
            .await
            .expect("healthz json body");
        assert_eq!(body.status, "ok");
        assert_eq!(body.connected_clients, 0);
        assert_eq!(body.messages_per_second, 0);
        assert_eq!(body.bytes_per_second, 0);
    })
    .await;
}

#[tokio::test]
async fn status_allows_clients_without_certificates() {
    with_relay(|state| async move {
        let client = browser_style_client();

        let response = client
            .get(state.config.uri())
            .header("Accept", "text/html")
            .send()
            .await
            .expect("browser-style status response");

        assert_eq!(response.status(), StatusCode::OK);
        let body = response.text().await.expect("status body");
        assert!(body.contains("Hoshi relay"));
        assert!(body.contains(&state.public_key));
        assert!(!body.contains("<!--PUBLIC_KEY-->"));
        assert!(!body.contains("<!--CONNECTED_CLIENTS-->"));
        assert!(!body.contains("<!--MESSAGES_PER_SECOND-->"));
        assert!(!body.contains("<!--BYTES_PER_SECOND-->"));
    })
    .await;
}

#[tokio::test]
async fn websocket_rejects_non_hoshi_user_agent() {
    with_relay(|state| async move {
        let identity = HoshiIdentity::generate();
        let client = ws_client(&identity);

        let response = client
            .get(state.config.uri())
            .header(USER_AGENT, "not-hoshi")
            .upgrade()
            .send()
            .await
            .expect("websocket response");

        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    })
    .await;
}

#[tokio::test]
async fn websocket_rejects_hoshi_user_agent_without_client_certificate() {
    with_relay(|state| async move {
        let client = browser_style_client();

        let response = client
            .get(state.config.uri())
            .header(USER_AGENT, "Hoshi relay test")
            .upgrade()
            .send()
            .await
            .expect("websocket response");

        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    })
    .await;
}

#[tokio::test]
async fn malformed_websocket_upgrade_returns_bad_request() {
    with_relay(|state| async move {
        let identity = HoshiIdentity::generate();
        let client = ws_client(&identity);

        let response = client
            .get(state.config.uri())
            .header(USER_AGENT, "Hoshi relay test")
            .header(UPGRADE, "websocket")
            .header(CONNECTION, "upgrade")
            .header(SEC_WEBSOCKET_VERSION, "13")
            .send()
            .await
            .expect("malformed websocket response");

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    })
    .await;
}

#[tokio::test]
async fn websocket_routes_envelopes_by_client_certificate_key() {
    with_relay(|state| async move {
        let sender_identity = HoshiIdentity::generate();
        let recipient_identity = HoshiIdentity::generate();
        let sender_client = ws_client(&sender_identity);
        let recipient_client = ws_client(&recipient_identity);

        let mut recipient_ws = recipient_client
            .get(state.config.uri())
            .header(USER_AGENT, "Hoshi relay test")
            .upgrade()
            .send()
            .await
            .expect("recipient upgrade response")
            .into_websocket()
            .await
            .expect("recipient websocket");

        let mut sender_ws = sender_client
            .get(state.config.uri())
            .header(USER_AGENT, "Hoshi relay test")
            .upgrade()
            .send()
            .await
            .expect("sender upgrade response")
            .into_websocket()
            .await
            .expect("sender websocket");

        let stats = wait_for_stats(state.config.uri(), |stats| stats.connected_clients == 2).await;
        assert_eq!(stats.connected_clients, 2);

        let envelope = HoshiEnvelope {
            recipient: recipient_identity.public_key_hex(),
            payload: b"hello recipient".to_vec(),
        };
        let bytes = rmp_serde::to_vec(&envelope).expect("serialize envelope");
        let sent_bytes = bytes.len() as u64;

        sender_ws
            .send(Message::Binary(bytes))
            .await
            .expect("send envelope");

        let received = timeout(Duration::from_secs(2), recipient_ws.next())
            .await
            .expect("recipient message timeout")
            .expect("recipient stream item")
            .expect("recipient websocket message");

        let Message::Binary(bytes) = received else {
            panic!("expected binary websocket message");
        };
        let routed =
            rmp_serde::from_slice::<HoshiEnvelope>(&bytes).expect("deserialize routed envelope");
        assert_eq!(routed.recipient, recipient_identity.public_key_hex());
        assert_eq!(routed.payload, b"hello recipient");

        let stats = wait_for_stats(state.config.uri(), |stats| {
            stats.messages_per_second >= 1 && stats.bytes_per_second >= sent_bytes
        })
        .await;
        assert!(stats.messages_per_second >= 1);
        assert!(stats.bytes_per_second >= sent_bytes);
    })
    .await;
}

fn ws_client(identity: &HoshiIdentity) -> reqwest::Client {
    reqwest::Client::builder()
        .use_preconfigured_tls(identity.make_client_tls_config())
        .build()
        .expect("websocket test client")
}

fn browser_style_client() -> reqwest::Client {
    reqwest::Client::builder()
        .danger_accept_invalid_certs(true)
        .build()
        .expect("browser-style test client")
}

async fn wait_for_stats(
    base_uri: String,
    predicate: impl Fn(&api::RelayStatusResponse) -> bool,
) -> api::RelayStatusResponse {
    let client = browser_style_client();
    let mut last = None;

    for _ in 0..40 {
        let stats = client
            .get(&base_uri)
            .send()
            .await
            .expect("stats response")
            .json::<api::RelayStatusResponse>()
            .await
            .expect("stats body");

        if predicate(&stats) {
            return stats;
        }

        last = Some(stats);
        sleep(Duration::from_millis(100)).await;
    }

    panic!("stats condition was not met; last stats: {last:?}");
}
