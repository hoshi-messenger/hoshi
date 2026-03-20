use hoshi_clientlib::identity::HoshiIdentity;
use reqwest::Client;

#[derive(Debug, Clone)]
pub struct RelayApi {
    base_uri: String,
    client: Client,
}

impl RelayApi {
    pub fn new(base_uri: impl Into<String>) -> Self {
        let identity = HoshiIdentity::generate();
        let tls_config = identity.make_client_tls_config();
        let client = Client::builder()
            .use_preconfigured_tls(tls_config)
            .build()
            .expect("failed to build test reqwest client");
        Self {
            base_uri: base_uri.into(),
            client,
        }
    }

    pub async fn get_index(&self) -> reqwest::Result<reqwest::Response> {
        self.client
            .get(format!("{}/", self.base_uri))
            .header("Accept", "text/html")
            .send()
            .await
    }

    pub async fn get_healthz(&self) -> reqwest::Result<reqwest::Response> {
        self.client.get(format!("{}/", self.base_uri)).send().await
    }
}
