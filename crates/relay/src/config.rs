use std::{
    fs,
    net::{IpAddr, Ipv4Addr, SocketAddr},
    path::PathBuf,
};

use anyhow::{Context, Result, anyhow};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use hoshi_protocol::noise;
use rand_core::{OsRng, RngCore};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone)]
pub struct Config {
    pub config_path: PathBuf,
    pub http_bind_address: SocketAddr,
    pub reuse_port: bool,
    pub control_plane_uri: String,
    pub guid: String,
    pub noise_static_private_key: String,
    pub api_key: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default)]
struct ConfigToml {
    http_bind_address: SocketAddr,
    reuse_port: bool,
    control_plane_uri: String,
    guid: String,
    noise_static_private_key: String,
    api_key: String,
}

impl Default for ConfigToml {
    fn default() -> Self {
        Self {
            http_bind_address: default_http_bind_address(),
            reuse_port: false,
            control_plane_uri: default_control_plane_uri(),
            guid: Uuid::now_v7().to_string(),
            noise_static_private_key: generate_noise_static_private_key(),
            api_key: String::new(),
        }
    }
}

fn default_config_path() -> PathBuf {
    dirs::home_dir()
        .map(|home| home.join(".hoshi").join("relay.toml"))
        .unwrap_or_else(|| PathBuf::from("./.hoshi/relay.toml"))
}

fn default_http_bind_address() -> SocketAddr {
    SocketAddr::new(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)), 2700)
}

fn default_control_plane_uri() -> String {
    if cfg!(debug_assertions) {
        "http://127.0.0.1:2600".to_string()
    } else {
        "https://cp.wikinarau.org".to_string()
    }
}

fn generate_noise_static_private_key() -> String {
    let mut key = [0_u8; 32];
    OsRng.fill_bytes(&mut key);
    STANDARD.encode(key)
}

impl ConfigToml {
    fn normalize(mut self) -> Result<Self> {
        self.control_plane_uri = if self.control_plane_uri.trim().is_empty() {
            default_control_plane_uri()
        } else {
            self.control_plane_uri.trim().to_string()
        };

        self.guid = if self.guid.trim().is_empty() {
            Uuid::now_v7().to_string()
        } else {
            Uuid::parse_str(self.guid.trim())
                .context("invalid guid")?
                .to_string()
        };

        self.noise_static_private_key = if self.noise_static_private_key.trim().is_empty() {
            generate_noise_static_private_key()
        } else {
            noise::canonicalize_base64_32(
                self.noise_static_private_key.trim(),
                "noise_static_private_key",
            )?
            .0
        };

        self.api_key = self.api_key.trim().to_string();

        Ok(self)
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
        .normalize()?;

        let serialized =
            toml::to_string_pretty(&file_config).context("failed to serialize relay config")?;
        fs::write(&config_path, format!("{serialized}\n")).with_context(|| {
            format!(
                "failed to write relay config file {}",
                config_path.display()
            )
        })?;

        if file_config.api_key.is_empty() {
            return Err(anyhow!(
                "missing api_key in relay config {}; set api_key and restart",
                config_path.display()
            ));
        }

        Ok(Self {
            config_path,
            http_bind_address: file_config.http_bind_address,
            reuse_port: file_config.reuse_port,
            control_plane_uri: file_config.control_plane_uri,
            guid: file_config.guid,
            noise_static_private_key: file_config.noise_static_private_key,
            api_key: file_config.api_key,
        })
    }

    pub fn update_bound_addresses(mut self, http_addr: SocketAddr) -> Self {
        self.http_bind_address = http_addr;
        self
    }

    pub fn uri(&self) -> String {
        format!("http://{}", self.http_bind_address)
    }
}
