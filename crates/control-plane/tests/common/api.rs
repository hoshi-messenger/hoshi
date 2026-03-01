pub use hoshi_control_plane::Client as ClientEntry;
pub use hoshi_control_plane::ClientType;
pub use hoshi_control_plane::api::{
    ErrorResponse, IssueRelayTokenRequest, IssueRelayTokenResponse, LookupClientResponse,
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
        req: &RegisterRelayRequest,
    ) -> reqwest::Result<reqwest::Response> {
        self.client
            .post(format!("{}/relays", self.base_uri))
            .json(req)
            .send()
            .await
    }

    pub async fn list_relays(&self) -> reqwest::Result<reqwest::Response> {
        self.client
            .get(format!("{}/relays", self.base_uri))
            .send()
            .await
    }

    pub async fn get_noise_public_key(&self) -> reqwest::Result<reqwest::Response> {
        self.client
            .get(format!("{}/noise/public-key", self.base_uri))
            .send()
            .await
    }

    pub async fn get_relay_jwt_public_key(&self) -> reqwest::Result<reqwest::Response> {
        self.client
            .get(format!("{}/auth/relay-jwt-public-key", self.base_uri))
            .send()
            .await
    }

    pub async fn issue_relay_token(
        &self,
        req: &IssueRelayTokenRequest,
    ) -> reqwest::Result<reqwest::Response> {
        self.client
            .post(format!("{}/auth/relay-token", self.base_uri))
            .json(req)
            .send()
            .await
    }
}
