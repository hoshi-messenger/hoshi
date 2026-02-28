use std::{sync::Arc, time::Instant};

use anyhow::Result;

use crate::Config;

#[derive(Debug, Clone)]
pub struct State {
    pub process_start: Instant,
    pub config: Arc<Config>,
}

impl State {
    /// Create a new GlobalState instance
    pub fn new(config: Config, process_start: Instant) -> Result<Self> {
        std::fs::create_dir_all(&config.dir_root)?;

        Ok(Self {
            process_start,
            config: Arc::new(config),
        })
    }
}