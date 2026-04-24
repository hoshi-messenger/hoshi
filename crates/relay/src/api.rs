use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RelayStatusResponse {
    pub status: String,
    pub public_key: String,
    pub connected_clients: u64,
    pub messages_per_second: u64,
    pub bytes_per_second: u64,
}
