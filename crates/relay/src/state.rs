use std::{
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};

use anyhow::Result;
use dashmap::DashMap;
use tokio::time::interval;

use crate::{Config, connection::HoshiConnection};

#[derive(Clone, Debug, Default)]
pub struct RelayStatsSnapshot {
    pub connected_clients: u64,
    pub messages_per_second: u64,
    pub bytes_per_second: u64,
}

#[derive(Debug, Default)]
pub struct RelayStats {
    connected_clients: AtomicU64,
    pending_messages: AtomicU64,
    pending_bytes: AtomicU64,
    messages_per_second: AtomicU64,
    bytes_per_second: AtomicU64,
}

impl RelayStats {
    pub fn client_connected(&self) {
        self.connected_clients.fetch_add(1, Ordering::Relaxed);
    }

    pub fn client_disconnected(&self) {
        self.connected_clients.fetch_sub(1, Ordering::Relaxed);
    }

    pub fn record_message(&self, bytes: u64) {
        self.pending_messages.fetch_add(1, Ordering::Relaxed);
        self.pending_bytes.fetch_add(bytes, Ordering::Relaxed);
    }

    pub fn snapshot(&self) -> RelayStatsSnapshot {
        RelayStatsSnapshot {
            connected_clients: self.connected_clients.load(Ordering::Relaxed),
            messages_per_second: self.messages_per_second.load(Ordering::Relaxed),
            bytes_per_second: self.bytes_per_second.load(Ordering::Relaxed),
        }
    }

    fn rotate_per_second_counters(&self) {
        let messages = self.pending_messages.swap(0, Ordering::Relaxed);
        let bytes = self.pending_bytes.swap(0, Ordering::Relaxed);
        self.messages_per_second.store(messages, Ordering::Relaxed);
        self.bytes_per_second.store(bytes, Ordering::Relaxed);
    }
}

#[derive(Clone)]
pub struct ServerState {
    pub process_start: std::time::Instant,
    pub config: Arc<Config>,
    pub connections: Arc<DashMap<String, Vec<HoshiConnection>>>,
    pub stats: Arc<RelayStats>,
    pub public_key: String,
}

impl ServerState {
    pub async fn new(
        config: Config,
        process_start: std::time::Instant,
        public_key: String,
    ) -> Result<Self> {
        let stats = Arc::new(RelayStats::default());
        spawn_stats_ticker(stats.clone());

        Ok(Self {
            process_start,
            config: Arc::new(config),
            connections: Arc::new(DashMap::new()),
            stats,
            public_key,
        })
    }
}

fn spawn_stats_ticker(stats: Arc<RelayStats>) {
    tokio::spawn(async move {
        let mut ticker = interval(Duration::from_secs(1));
        loop {
            ticker.tick().await;
            stats.rotate_per_second_counters();
        }
    });
}
