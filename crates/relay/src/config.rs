use std::{
    fs,
    net::{IpAddr, Ipv4Addr, SocketAddr},
    path::PathBuf,
};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone)]
pub struct Config {
    pub config_path: PathBuf,
    pub http_bind_address: SocketAddr,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default)]
struct ConfigToml {
    http_bind_address: SocketAddr,
}

impl Default for ConfigToml {
    fn default() -> Self {
        Self {
            http_bind_address: default_http_bind_address(),
        }
    }
}

fn default_config_path() -> PathBuf {
    dirs::home_dir()
        .map(|home| home.join(".hoshi").join("relay.toml"))
        .unwrap_or_else(|| PathBuf::from("./.hoshi/relay.toml"))
}

fn default_http_bind_address() -> SocketAddr {
    SocketAddr::new(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)), 2800)
}

impl ConfigToml {
    fn normalize(self) -> Self {
        self
    }
}

impl Config {
    pub fn new() -> Result<Self> {
        Self::load_from_path(default_config_path())
    }

    pub fn load_from_path(path: impl AsRef<std::path::Path>) -> Result<Self> {
        let config_path = path.as_ref().to_path_buf();
        if let Some(parent) = config_path.parent() {
            fs::create_dir_all(parent).with_context(|| {
                format!(
                    "failed to create relay config directory {}",
                    parent.display()
                )
            })?;
        }

        let file_config = if config_path.exists() {
            let raw = fs::read_to_string(&config_path).with_context(|| {
                format!("failed to read relay config file {}", config_path.display())
            })?;
            toml::from_str::<ConfigToml>(&raw).with_context(|| {
                format!(
                    "failed to parse relay config file {}",
                    config_path.display()
                )
            })?
        } else {
            ConfigToml::default()
        }
        .normalize();

        let serialized =
            toml::to_string_pretty(&file_config).context("failed to serialize relay config")?;
        fs::write(&config_path, format!("{serialized}\n")).with_context(|| {
            format!(
                "failed to write relay config file {}",
                config_path.display()
            )
        })?;

        Ok(Self {
            config_path,
            http_bind_address: file_config.http_bind_address,
        })
    }

    pub fn update_bound_addresses(mut self, http_addr: SocketAddr) -> Self {
        self.http_bind_address = http_addr;
        self
    }

    pub fn config_dir(&self) -> PathBuf {
        self.config_path
            .parent()
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| PathBuf::from("."))
    }

    pub fn private_key_path(&self) -> PathBuf {
        self.config_dir().join("relay.private_key")
    }

    pub fn uri(&self) -> String {
        format!("https://{}", self.http_bind_address)
    }
}
