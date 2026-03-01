use std::{sync::Arc, time::Instant};

use anyhow::{Result, anyhow};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use dashmap::DashMap;
use rand_core::{OsRng, RngCore};

use crate::{
    Config, RelayEntry,
    database::Database,
    noise::{
        canonicalize_base64_32, derive_public_key, encode_base64, generate_static_private_key,
    },
};

#[derive(Debug, Clone)]
pub struct ServerState {
    pub process_start: Instant,
    pub config: Arc<Config>,
    pub db: Database,
    pub relays: Arc<DashMap<String, RelayEntry>>,
    noise_static_private_key: Arc<[u8; 32]>,
    noise_public_key: Arc<String>,
}

impl ServerState {
    /// Create a new GlobalState instance
    pub async fn new(mut config: Config, process_start: Instant) -> Result<Self> {
        std::fs::create_dir_all(&config.dir_root)?;
        let db = Database::new(&config).await?;
        db.init().await?;

        if config.relay_api_key.is_none() {
            config.relay_api_key = db.get_relay_api_key().await?;
        }

        if config.relay_api_key.is_none() {
            let relay_api_key = generate_relay_api_key();
            db.set_relay_api_key(&relay_api_key).await?;
            config.relay_api_key = Some(relay_api_key);
        }

        if let Some(relay_api_key) = config.relay_api_key.as_deref() {
            db.set_relay_api_key(relay_api_key).await?;
        }

        if config.noise_static_private_key.is_none() {
            config.noise_static_private_key = db.get_noise_static_private_key().await?;
        }

        if config.noise_static_private_key.is_none() {
            let noise_private_key = generate_noise_static_private_key()?;
            db.set_noise_static_private_key(&noise_private_key).await?;
            config.noise_static_private_key = Some(noise_private_key);
        }

        let raw_noise_private_key = config
            .noise_static_private_key
            .as_deref()
            .ok_or_else(|| anyhow!("missing noise_static_private_key"))?;
        let (canonical_noise_private_key, noise_static_private_key) =
            canonicalize_base64_32(raw_noise_private_key, "noise_static_private_key")?;
        db.set_noise_static_private_key(&canonical_noise_private_key)
            .await?;
        config.noise_static_private_key = Some(canonical_noise_private_key);

        let noise_public_key = encode_base64(&derive_public_key(&noise_static_private_key));

        Ok(Self {
            process_start,
            config: Arc::new(config),
            db,
            relays: Arc::new(DashMap::new()),
            noise_static_private_key: Arc::new(noise_static_private_key),
            noise_public_key: Arc::new(noise_public_key),
        })
    }

    pub(crate) fn noise_static_private_key(&self) -> &[u8; 32] {
        self.noise_static_private_key.as_ref()
    }

    pub fn noise_public_key(&self) -> &str {
        self.noise_public_key.as_ref()
    }
}

fn generate_relay_api_key() -> String {
    let mut bytes = [0_u8; 32];
    OsRng.fill_bytes(&mut bytes);
    URL_SAFE_NO_PAD.encode(bytes)
}

fn generate_noise_static_private_key() -> Result<String> {
    Ok(encode_base64(&generate_static_private_key()?))
}
