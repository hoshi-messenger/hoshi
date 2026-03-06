use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RelayStatusResponse {
    pub status: String,
    pub public_key: String,
}
