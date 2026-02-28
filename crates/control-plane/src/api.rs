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
    pub guid: Option<String>,
}
