use std::{fmt, str::FromStr};

use anyhow::{Result, anyhow};

use crate::now;

#[derive(Debug, Clone)]
pub enum ClientType {
    User,
    Device,
    Relay,
}

#[derive(Debug, Clone)]
pub struct Client {
    pub id: String,
    pub owner_id: Option<String>,
    pub client_type: ClientType,
    pub public_key: Vec<u8>,
    pub created_at: i64,
    pub last_seen: i64,
}

impl Client {
    pub fn create_client(owner_id: Option<&str>, client_type: ClientType, public_key: &[u8]) -> Self {
        Self {
            id: uuid::Uuid::now_v7().to_string(),
            owner_id: owner_id.map(str::to_string),
            client_type,
            public_key: public_key.to_vec(),
            created_at: now(),
            last_seen: now(),
        }
    }
}

impl fmt::Display for ClientType {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            ClientType::User => write!(f, "user"),
            ClientType::Device => write!(f, "device"),
            ClientType::Relay => write!(f, "relay"),
        }
    }
}

impl FromStr for ClientType {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self> {
        match s {
            "user" => Ok(ClientType::User),
            "device" => Ok(ClientType::Device),
            "relay" => Ok(ClientType::Relay),
            other => Err(anyhow!("Unknown client type: {}", other)),
        }
    }
}