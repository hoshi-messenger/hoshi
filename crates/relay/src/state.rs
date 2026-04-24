use std::sync::Arc;

use anyhow::Result;
use dashmap::DashMap;

use crate::{Config, connection::HoshiConnection};

#[derive(Clone)]
pub struct ServerState {
    pub process_start: std::time::Instant,
    pub config: Arc<Config>,
    pub connections: Arc<DashMap<String, Vec<HoshiConnection>>>,
    pub public_key: String,
}

impl ServerState {
    pub async fn new(
        config: Config,
        process_start: std::time::Instant,
        public_key: String,
    ) -> Result<Self> {
        Ok(Self {
            process_start,
            config: Arc::new(config),
            connections: Arc::new(DashMap::new()),
            public_key,
        })
    }
}
