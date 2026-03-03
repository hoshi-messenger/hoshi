use std::{sync::Arc, time::Instant};

use anyhow::Result;
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use dashmap::DashMap;
use rand_core::{OsRng, RngCore};

use crate::{Config, api, database::Database};

#[derive(Debug, Clone)]
pub struct RelayPresence {
    pub entry: api::RelayEntry,
    pub last_seen: i64,
}

#[derive(Clone)]
pub struct ServerState {
    pub process_start: Instant,
    pub config: Arc<Config>,
    pub db: Database,
    pub relays: Arc<DashMap<String, RelayPresence>>,
}

impl ServerState {
    /// Create a new GlobalState instance
    pub async fn new(mut config: Config, process_start: Instant) -> Result<Self> {
        std::fs::create_dir_all(&config.dir_root)?;
        let db = Database::new(&config).await?;
        db.init().await?;

        Self::initialize_relay_api_key(&mut config, &db).await?;

        Ok(Self {
            process_start,
            config: Arc::new(config),
            db,
            relays: Arc::new(DashMap::new()),
        })
    }

    async fn initialize_relay_api_key(config: &mut Config, db: &Database) -> Result<()> {
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

        Ok(())
    }
}

fn generate_relay_api_key() -> String {
    let mut bytes = [0_u8; 32];
    OsRng.fill_bytes(&mut bytes);
    URL_SAFE_NO_PAD.encode(bytes)
}
