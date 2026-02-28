pub use hoshi_control_plane::Client as ClientEntry;
pub use hoshi_control_plane::ClientType;
pub use hoshi_control_plane::api::{
    ErrorResponse, LookupClientResponse, RegisterClientRequest, RegisterRelayRequest,
};
use reqwest::Client;

#[derive(Debug, Clone)]
pub struct ControlPlaneApi {
    base_uri: String,
    client: Client,
}

impl ControlPlaneApi {
    pub fn new(base_uri: impl Into<String>) -> Self {
        Self {
            base_uri: base_uri.into(),
            client: Client::new(),
        }
    }

    pub async fn register_client(
        &self,
        req: &RegisterClientRequest,
    ) -> reqwest::Result<reqwest::Response> {
        self.client
            .post(format!("{}/clients", self.base_uri))
            .json(req)
            .send()
            .await
    }

    pub async fn lookup_client(&self, guid: &str) -> reqwest::Result<reqwest::Response> {
        self.client
            .get(format!("{}/clients/{}", self.base_uri, guid))
            .send()
            .await
    }

    pub async fn register_relay(
        &self,
        api_key: Option<&str>,
        req: &RegisterRelayRequest,
    ) -> reqwest::Result<reqwest::Response> {
        let mut request = self
            .client
            .post(format!("{}/relays", self.base_uri))
            .json(req);

        if let Some(api_key) = api_key {
            request = request.header("x-api-key", api_key);
        }

        request.send().await
    }
}
