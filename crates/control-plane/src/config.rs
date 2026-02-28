use std::{
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
        }
    }
}

impl Config {
    pub fn new() -> Result<Self> {
        let dir_root = dirs::home_dir()
            .map(|h| h.join(".hoshi"))
            .unwrap_or_else(|| PathBuf::from("./.hoshi"));

        let ip = IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1));
        let port = 2600;
        let http_bind_address = SocketAddr::new(ip, port);
        let db_name = "control_plane.sqlite3".to_string();

        Ok(Self {
            dir_root,
            http_bind_address,
            reuse_port: false,
            db_name,
        })
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
        format!("http://{}", self.http_bind_address.to_string())
    }

    pub fn set_db_name(mut self, db_name: &str) -> Self {
        self.db_name = db_name.to_string();
        self
    }
}
