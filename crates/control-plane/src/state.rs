use std::{sync::Arc, time::Instant};

use anyhow::{Context, Result, anyhow};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use dashmap::DashMap;
use ed25519_dalek::{SigningKey, pkcs8::EncodePrivateKey};
use jsonwebtoken::EncodingKey;
use rand_core::{OsRng, RngCore};

use crate::{
    Config, RelayEntry,
    database::Database,
    noise::{
        canonicalize_base64_32, derive_public_key, encode_base64, generate_static_private_key,
    },
};

#[derive(Clone)]
pub struct ServerState {
    pub process_start: Instant,
    pub config: Arc<Config>,
    pub db: Database,
    pub relays: Arc<DashMap<String, RelayEntry>>,
    noise_static_private_key: Arc<[u8; 32]>,
    noise_public_key: Arc<String>,
    relay_jwt_encoding_key: Arc<EncodingKey>,
    relay_jwt_public_key_x: Arc<String>,
}

impl ServerState {
    /// Create a new GlobalState instance
    pub async fn new(mut config: Config, process_start: Instant) -> Result<Self> {
        std::fs::create_dir_all(&config.dir_root)?;
        let db = Database::new(&config).await?;
        db.init().await?;

        Self::initialize_relay_api_key(&mut config, &db).await?;
        let noise_static_private_key =
            Self::initialize_noise_static_private_key(&mut config, &db).await?;
        let noise_public_key = encode_base64(&derive_public_key(&noise_static_private_key));
        let relay_jwt_signing_private_key =
            Self::initialize_relay_jwt_signing_private_key(&mut config, &db).await?;
        let (relay_jwt_encoding_key, relay_jwt_public_key_x) =
            build_relay_jwt_signing_material(&relay_jwt_signing_private_key)?;

        Ok(Self {
            process_start,
            config: Arc::new(config),
            db,
            relays: Arc::new(DashMap::new()),
            noise_static_private_key: Arc::new(noise_static_private_key),
            noise_public_key: Arc::new(noise_public_key),
            relay_jwt_encoding_key: Arc::new(relay_jwt_encoding_key),
            relay_jwt_public_key_x: Arc::new(relay_jwt_public_key_x),
        })
    }

    async fn initialize_relay_api_key(config: &mut Config, db: &Database) -> Result<()> {
        if config.relay_api_key.is_none() {
            config.relay_api_key = db.get_relay_api_key().await?;
        }

        if config.relay_api_key.is_none() {
            let relay_api_key = generate_relay_api_key();
            db.set_relay_api_key(&relay_api_key).await?;
            config.relay_api_key = Some(relay_api_key);
        }

        if let Some(relay_api_key) = config.relay_api_key.as_deref() {
            db.set_relay_api_key(relay_api_key).await?;
        }

        Ok(())
    }

    async fn initialize_noise_static_private_key(
        config: &mut Config,
        db: &Database,
    ) -> Result<[u8; 32]> {
        if config.noise_static_private_key.is_none() {
            config.noise_static_private_key = db.get_noise_static_private_key().await?;
        }

        if config.noise_static_private_key.is_none() {
            let noise_private_key = generate_noise_static_private_key()?;
            db.set_noise_static_private_key(&noise_private_key).await?;
            config.noise_static_private_key = Some(noise_private_key);
        }

        let raw_noise_private_key = config
            .noise_static_private_key
            .as_deref()
            .ok_or_else(|| anyhow!("missing noise_static_private_key"))?;
        let (canonical_noise_private_key, noise_static_private_key) =
            canonicalize_base64_32(raw_noise_private_key, "noise_static_private_key")?;
        db.set_noise_static_private_key(&canonical_noise_private_key)
            .await?;
        config.noise_static_private_key = Some(canonical_noise_private_key);

        Ok(noise_static_private_key)
    }

    async fn initialize_relay_jwt_signing_private_key(
        config: &mut Config,
        db: &Database,
    ) -> Result<[u8; 32]> {
        if config.relay_jwt_signing_private_key.is_none() {
            config.relay_jwt_signing_private_key = db.get_relay_jwt_signing_private_key().await?;
        }

        if config.relay_jwt_signing_private_key.is_none() {
            let relay_jwt_signing_private_key = generate_relay_jwt_signing_private_key();
            db.set_relay_jwt_signing_private_key(&relay_jwt_signing_private_key)
                .await?;
            config.relay_jwt_signing_private_key = Some(relay_jwt_signing_private_key);
        }

        let raw_relay_jwt_signing_private_key = config
            .relay_jwt_signing_private_key
            .as_deref()
            .ok_or_else(|| anyhow!("missing relay_jwt_signing_private_key"))?;
        let (canonical_relay_jwt_signing_private_key, relay_jwt_signing_private_key) =
            canonicalize_base64_32(
                raw_relay_jwt_signing_private_key,
                "relay_jwt_signing_private_key",
            )?;
        db.set_relay_jwt_signing_private_key(&canonical_relay_jwt_signing_private_key)
            .await?;
        config.relay_jwt_signing_private_key = Some(canonical_relay_jwt_signing_private_key);

        Ok(relay_jwt_signing_private_key)
    }

    pub(crate) fn noise_static_private_key(&self) -> &[u8; 32] {
        self.noise_static_private_key.as_ref()
    }

    pub fn noise_public_key(&self) -> &str {
        self.noise_public_key.as_ref()
    }

    pub(crate) fn relay_jwt_encoding_key(&self) -> &EncodingKey {
        self.relay_jwt_encoding_key.as_ref()
    }

    pub fn relay_jwt_public_key_x(&self) -> &str {
        self.relay_jwt_public_key_x.as_ref()
    }
}

fn generate_relay_api_key() -> String {
    let mut bytes = [0_u8; 32];
    OsRng.fill_bytes(&mut bytes);
    URL_SAFE_NO_PAD.encode(bytes)
}

fn generate_noise_static_private_key() -> Result<String> {
    Ok(encode_base64(&generate_static_private_key()?))
}

fn generate_relay_jwt_signing_private_key() -> String {
    let mut bytes = [0_u8; 32];
    OsRng.fill_bytes(&mut bytes);
    encode_base64(&bytes)
}

fn build_relay_jwt_signing_material(private_key: &[u8; 32]) -> Result<(EncodingKey, String)> {
    let signing_key = SigningKey::from_bytes(private_key);
    let verifying_key = signing_key.verifying_key();
    let private_key_der = signing_key
        .to_pkcs8_der()
        .context("failed to encode relay jwt signing key")?;
    let encoding_key = EncodingKey::from_ed_der(private_key_der.as_bytes());
    let public_key_x = URL_SAFE_NO_PAD.encode(verifying_key.to_bytes());

    Ok((encoding_key, public_key_x))
}
