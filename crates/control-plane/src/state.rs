use std::{sync::Arc, time::Instant};

use anyhow::Result;

use crate::{Config, database::Database};

#[derive(Debug, Clone)]
pub struct ServerState {
    pub process_start: Instant,
    pub config: Arc<Config>,
    pub db: Database,
}

impl ServerState {
    /// Create a new GlobalState instance
    pub fn new(config: Config, process_start: Instant) -> Result<Self> {
        std::fs::create_dir_all(&config.dir_root)?;
        let db = Database::new(&config)?;

        Ok(Self {
            process_start,
            config: Arc::new(config),
            db,
        })
    }
}
