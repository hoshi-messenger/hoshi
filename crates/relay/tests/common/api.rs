pub use hoshi_relay::api::HealthzResponse;
use reqwest::Client;

#[derive(Debug, Clone)]
pub struct RelayApi {
    base_uri: String,
    client: Client,
}

impl RelayApi {
    pub fn new(base_uri: impl Into<String>) -> Self {
        Self {
            base_uri: base_uri.into(),
            client: Client::new(),
        }
    }

    pub async fn get_index(&self) -> reqwest::Result<reqwest::Response> {
        self.client.get(format!("{}/", self.base_uri)).send().await
    }

    pub async fn get_healthz(&self) -> reqwest::Result<reqwest::Response> {
        self.client
            .get(format!("{}/healthz", self.base_uri))
            .send()
            .await
    }
}
