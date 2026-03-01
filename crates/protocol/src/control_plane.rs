use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ClientType {
    User,
    Device,
    Relay,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClientEntry {
    pub id: String,
    pub client_type: ClientType,
    pub public_key: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegisterClientRequest {
    pub public_key: String,
    pub owner_id: Option<String>,
    pub client_type: ClientType,
    pub noise_handshake: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LookupClientResponse {
    pub public_key: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegisterRelayRequest {
    pub public_key: String,
    pub guid: String,
    pub api_key: String,
    pub port: u16,
    pub noise_handshake: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NoisePublicKeyResponse {
    pub pattern: String,
    pub public_key: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RelayJwtPublicKeyResponse {
    pub alg: String,
    pub x: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IssueRelayTokenRequest {
    pub public_key: String,
    pub noise_handshake: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IssueRelayTokenResponse {
    pub token: String,
    pub expires_at: i64,
    pub client_guid: String,
    pub device_guid: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RelayEntry {
    pub guid: String,
    pub public_key: String,
    pub ip: String,
    pub port: u16,
}
