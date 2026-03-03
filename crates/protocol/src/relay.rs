use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct HealthzResponse {
    pub status: String,
    pub public_key: String,
    pub control_plane_uri: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RelayPacket {
    pub recipient: String,
    pub payload: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct E2eeEnvelope {
    pub version: u8,
    pub alg: String,
    pub ciphertext: String,
}
