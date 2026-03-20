use std::sync::Arc;

use anyhow::{Context, Result};
use hoshi_clientlib::identity::HoshiIdentity;
use tokio_rustls::TlsAcceptor;

use crate::Config;

pub fn load_or_generate_identity(config: &Config) -> Result<HoshiIdentity> {
    let key_path = config.private_key_path();

    if key_path.exists() {
        let seed_bytes = std::fs::read(&key_path)
            .with_context(|| format!("failed to read private key from {}", key_path.display()))?;
        let seed: [u8; 32] = seed_bytes
            .try_into()
            .map_err(|_| anyhow::anyhow!("private key must be exactly 32 bytes"))?;
        println!("Loaded relay private key from {}", key_path.display());
        Ok(HoshiIdentity::from_seed(seed))
    } else {
        let identity = HoshiIdentity::generate();
        std::fs::write(&key_path, identity.seed())
            .with_context(|| format!("failed to write private key to {}", key_path.display()))?;

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&key_path, std::fs::Permissions::from_mode(0o600))
                .with_context(|| "failed to set private key permissions")?;
        }

        println!("Generated new relay private key at {}", key_path.display());
        Ok(identity)
    }
}

pub fn create_tls_acceptor(identity: &HoshiIdentity) -> Result<TlsAcceptor> {
    let server_config = identity.make_server_tls_config();
    Ok(TlsAcceptor::from(Arc::new(server_config)))
}
