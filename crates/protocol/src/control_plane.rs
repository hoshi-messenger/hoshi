use serde::{Deserialize, Serialize};


#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClientEntry {
    pub public_key: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegisterClientRequest {
    pub public_key: String,
    pub noise_handshake: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ClientRegistrationProofPayload {
    pub public_key: String,
}


#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegisterRelayRequest {
    pub public_key: String,
    pub noise_handshake: String,

    pub api_key: String,
    pub port: u16,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RelayRegistrationProofPayload {
    pub public_key: String,
    pub api_key: String,
    pub port: u16,
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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RelayTokenProofPayload {
    pub public_key: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IssueRelayTokenResponse {
    pub token: String,
    pub expires_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RelayEntry {
    pub public_key: String,
    pub ip: String,
    pub port: u16,
}
