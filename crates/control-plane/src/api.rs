use serde::{Deserialize, Serialize};

use crate::{Client, ClientType};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ErrorResponse {
    pub error: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegisterClientRequest {
    pub public_key: String,
    pub owner_id: Option<String>,
    pub client_type: ClientType,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LookupClientResponse {
    pub client: Client,
    pub children: Vec<Client>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegisterRelayRequest {
    pub public_key: String,
    pub guid: String,
    pub api_key: String,
    pub port: u16,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RelayEntry {
    pub guid: String,
    pub public_key: String,
    pub ip: String,
    pub port: u16,
}
