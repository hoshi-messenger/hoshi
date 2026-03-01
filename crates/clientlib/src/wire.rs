use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Deserialize)]
pub struct ErrorResponse {
    pub error: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct NoisePublicKeyResponse {
    pub public_key: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct IssueRelayTokenRequest {
    pub public_key: String,
    pub noise_handshake: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct IssueRelayTokenResponse {
    pub token: String,
    pub client_guid: String,
    pub device_guid: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct RelayEntry {
    pub guid: String,
    pub public_key: String,
    pub ip: String,
    pub port: u16,
}

#[derive(Debug, Clone, Deserialize)]
pub struct LookupClientResponse {
    pub client: ClientEntry,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ClientEntry {
    pub public_key: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RelayPacket {
    pub recipient: String,
    pub payload: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RelayErrorPacket {
    pub error: String,
    pub recipient: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct E2eeEnvelope {
    pub version: u8,
    pub alg: String,
    pub ciphertext: String,
}
