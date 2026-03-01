mod auth;
mod registration;
mod sessions;

use std::{
    sync::{Arc, atomic::AtomicU64},
    time::Duration,
};

use anyhow::{Context, Result};
use dashmap::{DashMap, DashSet};
use jsonwebtoken::DecodingKey;
use tokio::sync::RwLock;

pub use sessions::{ConnectionIdentity, OutboundCommand};

use crate::{
    Config,
    noise::{canonicalize_base64_32, derive_public_key, encode_base64},
};

#[derive(Clone)]
pub struct ServerState {
    pub process_start: std::time::Instant,
    pub config: Arc<Config>,
    pub http_client: reqwest::Client,
    noise_static_private_key: Arc<[u8; 32]>,
    noise_public_key: Arc<String>,
    relay_jwt_decoding_key: Arc<RwLock<Option<DecodingKey>>>,
    device_sessions: Arc<DashMap<String, sessions::SessionHandle>>,
    client_devices: Arc<DashMap<String, DashSet<String>>>,
    next_session_id: Arc<AtomicU64>,
}

impl ServerState {
    pub async fn new(config: Config, process_start: std::time::Instant) -> Result<Self> {
        let http_client = reqwest::Client::builder()
            .timeout(Duration::from_secs(3))
            .build()
            .context("failed to build relay http client")?;

        let (_, noise_static_private_key) =
            canonicalize_base64_32(&config.noise_static_private_key, "noise_static_private_key")?;
        let noise_public_key = encode_base64(&derive_public_key(&noise_static_private_key));

        Ok(Self {
            process_start,
            config: Arc::new(config),
            http_client,
            noise_static_private_key: Arc::new(noise_static_private_key),
            noise_public_key: Arc::new(noise_public_key),
            relay_jwt_decoding_key: Arc::new(RwLock::new(None)),
            device_sessions: Arc::new(DashMap::new()),
            client_devices: Arc::new(DashMap::new()),
            next_session_id: Arc::new(AtomicU64::new(0)),
        })
    }

    pub async fn probe_control_plane(&self) -> Result<reqwest::StatusCode> {
        let cp_uri = self.config.control_plane_uri.trim_end_matches('/');
        let endpoint = format!("{cp_uri}/noise/public-key");

        let response = self
            .http_client
            .get(&endpoint)
            .send()
            .await
            .with_context(|| format!("control-plane probe failed: {endpoint}"))?;

        Ok(response.status())
    }

    pub fn noise_static_private_key(&self) -> &[u8; 32] {
        self.noise_static_private_key.as_ref()
    }

    pub fn noise_public_key(&self) -> &str {
        self.noise_public_key.as_ref()
    }
}
