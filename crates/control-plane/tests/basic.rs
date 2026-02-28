mod common;

use common::with_backend;
use reqwest::Client;

#[tokio::test]
async fn basic_http() {
    with_backend(|state| async move {
        let client = Client::builder()
            .build()
            .expect("Failed to create reqwest client");

        let base = state.config.uri();

        let res = client.get(format!("{}/", base))
            .send()
            .await
            .unwrap();

        let text = res.text().await.unwrap();
        assert!(
            text.contains("Hoshi"),
            "Hoshi doesn't appear on landing page"
        );
    })
    .await;
}
