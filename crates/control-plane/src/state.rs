use std::{sync::Arc, time::Instant};

use anyhow::Result;
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use dashmap::DashMap;
use rand_core::{OsRng, RngCore};

use crate::{Config, RelayEntry, database::Database};

#[derive(Debug, Clone)]
pub struct ServerState {
    pub process_start: Instant,
    pub config: Arc<Config>,
    pub db: Database,
    pub relays: Arc<DashMap<String, RelayEntry>>,
}

impl ServerState {
    /// Create a new GlobalState instance
    pub fn new(mut config: Config, process_start: Instant) -> Result<Self> {
        std::fs::create_dir_all(&config.dir_root)?;
        let db = Database::new(&config)?;
        db.init()?;

        if config.relay_api_key.is_none() {
            config.relay_api_key = db.get_relay_api_key()?;
        }

        if config.relay_api_key.is_none() {
            let relay_api_key = generate_relay_api_key();
            db.set_relay_api_key(&relay_api_key)?;
            config.relay_api_key = Some(relay_api_key);
        }

        if let Some(relay_api_key) = config.relay_api_key.as_deref() {
            db.set_relay_api_key(relay_api_key)?;
        }

        Ok(Self {
            process_start,
            config: Arc::new(config),
            db,
            relays: Arc::new(DashMap::new()),
        })
    }
}

fn generate_relay_api_key() -> String {
    let mut bytes = [0_u8; 32];
    OsRng.fill_bytes(&mut bytes);
    URL_SAFE_NO_PAD.encode(bytes)
}
