use std::{sync::Arc, time::Duration};

use anyhow::{Context, Result};

use crate::Config;

#[derive(Debug, Clone)]
pub struct ServerState {
    pub process_start: std::time::Instant,
    pub config: Arc<Config>,
    pub http_client: reqwest::Client,
}

impl ServerState {
    pub async fn new(config: Config, process_start: std::time::Instant) -> Result<Self> {
        let http_client = reqwest::Client::builder()
            .timeout(Duration::from_secs(3))
            .build()
            .context("failed to build relay http client")?;

        Ok(Self {
            process_start,
            config: Arc::new(config),
            http_client,
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
}
