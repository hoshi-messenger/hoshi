mod common;

use common::{RelayApi, with_relay};
use hoshi_relay::api;
use reqwest::StatusCode;

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
    })
    .await;
}
