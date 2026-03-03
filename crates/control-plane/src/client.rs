use serde::{Deserialize, Serialize};

use crate::now;
use crate::api;


#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Client {
    pub public_key: String,
    pub created_at: i64,
    pub last_seen: i64,
}

impl Client {
    pub fn create_client(public_key: &str) -> Self {
        Self {
            public_key: public_key.to_string(),
            created_at: now(),
            last_seen: now(),
        }
    }
}

impl From<&Client> for api::ClientEntry {
    fn from(value: &Client) -> Self {
        Self {
            public_key: value.public_key.clone(),
        }
    }
}
