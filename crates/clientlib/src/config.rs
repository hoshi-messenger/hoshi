use std::{fs, path::PathBuf};

use anyhow::{Context, Result};
use rand_core::{OsRng, RngCore};
use serde::{Deserialize, Serialize};

use crate::noise::{canonicalize_base64_32, decode_base64_32, encode_base64};

#[derive(Debug, Clone)]
pub struct ClientConfig {
    pub config_path: PathBuf,
    pub control_plane_uri: String,
    pub noise_static_private_key: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default)]
struct ClientConfigToml {
    control_plane_uri: String,
    noise_static_private_key: String,
}

impl Default for ClientConfigToml {
    fn default() -> Self {
        Self {
            control_plane_uri: default_control_plane_uri(),
            noise_static_private_key: generate_noise_static_private_key(),
        }
    }
}

fn default_control_plane_uri() -> String {
    if cfg!(debug_assertions) {
        "http://127.0.0.1:2600".to_string()
    } else {
        "https://cp.wikinarau.org".to_string()
    }
}

fn default_config_path() -> PathBuf {
    dirs::home_dir()
        .map(|home| home.join(".hoshi").join("client.toml"))
        .unwrap_or_else(|| PathBuf::from("./.hoshi/client.toml"))
}

fn generate_noise_static_private_key() -> String {
    let mut key = [0_u8; 32];
    OsRng.fill_bytes(&mut key);
    encode_base64(&key)
}

impl ClientConfigToml {
    fn normalize(mut self) -> Result<Self> {
        self.control_plane_uri = if self.control_plane_uri.trim().is_empty() {
            default_control_plane_uri()
        } else {
            self.control_plane_uri.trim().to_string()
        };

        self.noise_static_private_key = if self.noise_static_private_key.trim().is_empty() {
            generate_noise_static_private_key()
        } else {
            canonicalize_base64_32(
                self.noise_static_private_key.trim(),
                "noise_static_private_key",
            )?
        };

        Ok(self)
    }
}

impl ClientConfig {
    pub fn new() -> Result<Self> {
        Self::load_from_path(default_config_path())
    }

    pub fn load_from_path(path: impl AsRef<std::path::Path>) -> Result<Self> {
        let config_path = path.as_ref().to_path_buf();
        if let Some(parent) = config_path.parent() {
            fs::create_dir_all(parent).with_context(|| {
                format!(
                    "failed to create client config directory {}",
                    parent.display()
                )
            })?;
        }

        let file_config = if config_path.exists() {
            let raw = fs::read_to_string(&config_path).with_context(|| {
                format!(
                    "failed to read client config file {}",
                    config_path.display()
                )
            })?;
            toml::from_str::<ClientConfigToml>(&raw).with_context(|| {
                format!(
                    "failed to parse client config file {}",
                    config_path.display()
                )
            })?
        } else {
            ClientConfigToml::default()
        }
        .normalize()?;

        let serialized =
            toml::to_string_pretty(&file_config).context("failed to serialize client config")?;
        fs::write(&config_path, format!("{serialized}\n")).with_context(|| {
            format!(
                "failed to write client config file {}",
                config_path.display()
            )
        })?;

        Ok(Self {
            config_path,
            control_plane_uri: file_config.control_plane_uri,
            noise_static_private_key: file_config.noise_static_private_key,
        })
    }

    pub(crate) fn noise_static_private_key_bytes(&self) -> Result<[u8; 32]> {
        decode_base64_32(&self.noise_static_private_key, "noise_static_private_key")
    }
}
