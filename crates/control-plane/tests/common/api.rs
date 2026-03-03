pub use hoshi_protocol::control_plane::{
    ClientEntry, IssueRelayTokenRequest, IssueRelayTokenResponse, 
    NoisePublicKeyResponse, RegisterClientRequest, RegisterRelayRequest, RelayEntry,
    RelayJwtPublicKeyResponse,
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

    pub async fn list_relays(&self) -> reqwest::Result<reqwest::Response> {
        self.client
            .get(format!("{}/relays", self.base_uri))
            .send()
            .await
    }
}
