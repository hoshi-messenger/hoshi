use std::{
    env,
    net::{IpAddr, Ipv4Addr, SocketAddr},
    path::PathBuf,
};

use anyhow::Result;

#[derive(Debug, Clone)]
pub struct Config {
    pub dir_root: PathBuf,
    pub http_bind_address: SocketAddr,
    pub reuse_port: bool,
    pub db_name: String,
    pub relay_api_key: Option<String>,
}

fn relay_api_key_from_env() -> Option<String> {
    env::var("HOSHI_RELAY_API_KEY")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

impl Default for Config {
    fn default() -> Self {
        let dir_root = dirs::home_dir()
            .map(|h| h.join(".hoshi"))
            .unwrap_or_else(|| PathBuf::from("./.hoshi"));

        let ip = IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1));
        let port = 2600;
        let http_bind_address = SocketAddr::new(ip, port);
        let db_name = "control_plane.sqlite3".to_string();

        Self {
            dir_root,
            http_bind_address,
            reuse_port: false,
            db_name,
            relay_api_key: relay_api_key_from_env(),
        }
    }
}

impl Config {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn update_bound_addresses(mut self, http_addr: SocketAddr) -> Self {
        self.http_bind_address = http_addr;
        self
    }

    pub fn set_dir_root(mut self, dir_root: &str) -> Self {
        let dir_root = PathBuf::from(dir_root);
        self.dir_root = dir_root;
        self
    }

    pub fn set_http_bind_addr(mut self, bind_addr: &str) -> Result<Self> {
        let addr = bind_addr.parse::<SocketAddr>()?;
        self.http_bind_address = addr;
        Ok(self)
    }

    pub fn uri(&self) -> String {
        format!("http://{}", self.http_bind_address)
    }

    pub fn set_db_name(mut self, db_name: &str) -> Self {
        self.db_name = db_name.to_string();
        self
    }

    pub fn set_relay_api_key(mut self, relay_api_key: &str) -> Self {
        self.relay_api_key = Some(relay_api_key.to_string());
        self
    }
}
