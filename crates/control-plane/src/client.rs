use std::{fmt, str::FromStr};

use anyhow::{Result, anyhow};
use hoshi_protocol::control_plane as protocol;
use serde::{Deserialize, Serialize};

use crate::now;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ClientType {
    User,
    Device,
    Relay,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Client {
    pub id: String,
    pub client_type: ClientType,
    pub public_key: String,
    pub created_at: i64,
    pub last_seen: i64,
}

impl Client {
    pub fn create_client(client_type: ClientType, public_key: &str) -> Self {
        Self {
            id: uuid::Uuid::now_v7().to_string(),
            client_type,
            public_key: public_key.to_string(),
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

impl From<protocol::ClientType> for ClientType {
    fn from(value: protocol::ClientType) -> Self {
        match value {
            protocol::ClientType::User => ClientType::User,
            protocol::ClientType::Device => ClientType::Device,
            protocol::ClientType::Relay => ClientType::Relay,
        }
    }
}

impl From<ClientType> for protocol::ClientType {
    fn from(value: ClientType) -> Self {
        match value {
            ClientType::User => protocol::ClientType::User,
            ClientType::Device => protocol::ClientType::Device,
            ClientType::Relay => protocol::ClientType::Relay,
        }
    }
}

impl From<&Client> for protocol::ClientEntry {
    fn from(value: &Client) -> Self {
        Self {
            id: value.id.clone(),
            client_type: value.client_type.clone().into(),
            public_key: value.public_key.clone(),
        }
    }
}
