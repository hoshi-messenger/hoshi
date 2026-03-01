use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct HealthzResponse {
    pub status: String,
    pub guid: String,
    pub control_plane_uri: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ErrorResponse {
    pub error: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NoisePublicKeyResponse {
    pub pattern: String,
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
pub struct RelayJwtPublicKeyResponse {
    pub alg: String,
    pub x: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RelayPacket {
    pub recipient: String,
    pub payload: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RelayErrorPacket {
    pub error: String,
    pub recipient: Option<String>,
}
