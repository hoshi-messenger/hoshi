use std::{sync::Arc, time::Duration};

use anyhow::{Context, Result};
use dashmap::DashMap;

use crate::{Config, connection::HoshiConnection};

#[derive(Clone)]
pub struct ServerState {
    pub process_start: std::time::Instant,
    pub config: Arc<Config>,
    pub http_client: reqwest::Client,
    pub connections: Arc<DashMap<String, Vec<HoshiConnection>>>,
    pub public_key: String,
}

impl ServerState {
    pub async fn new(
        config: Config,
        process_start: std::time::Instant,
        public_key: String,
    ) -> Result<Self> {
        let http_client = reqwest::Client::builder()
            .timeout(Duration::from_secs(3))
            .build()
            .context("failed to build relay http client")?;

        Ok(Self {
            process_start,
            config: Arc::new(config),
            http_client,
            connections: Arc::new(DashMap::new()),
            public_key,
        })
    }
}
