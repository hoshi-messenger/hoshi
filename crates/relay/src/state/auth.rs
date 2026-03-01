use std::time::Duration;

use anyhow::{Context, Result, anyhow};
use hoshi_protocol::control_plane::{ClientType, RelayJwtPublicKeyResponse};
use jsonwebtoken::{Algorithm, DecodingKey, Validation, decode};
use serde::Deserialize;
use uuid::Uuid;

use super::{ConnectionIdentity, ServerState};

const JWT_KEY_REFRESH_SUCCESS_INTERVAL: Duration = Duration::from_secs(300);
const JWT_KEY_REFRESH_RETRY_INTERVAL: Duration = Duration::from_secs(10);

#[derive(Debug, Deserialize)]
struct RelayJwtClaims {
    sub: String,
    exp: i64,
    client_type: ClientType,
}

impl ServerState {
    pub async fn relay_jwt_ready(&self) -> bool {
        self.relay_jwt_decoding_key.read().await.is_some()
    }

    pub async fn refresh_relay_jwt_decoding_key_once(&self) -> Result<()> {
        let cp_uri = self.config.control_plane_uri.trim_end_matches('/');
        let endpoint = format!("{cp_uri}/auth/relay-jwt-public-key");

        let response = self
            .http_client
            .get(&endpoint)
            .send()
            .await
            .with_context(|| format!("failed to fetch relay jwt key: {endpoint}"))?;

        if !response.status().is_success() {
            return Err(anyhow!(
                "relay jwt key fetch returned non-success status: {}",
                response.status()
            ));
        }

        let body = response
            .json::<RelayJwtPublicKeyResponse>()
            .await
            .context("failed to decode relay jwt key payload")?;

        if body.alg != "EdDSA" {
            return Err(anyhow!("unsupported relay jwt algorithm: {}", body.alg));
        }

        let decoding_key =
            DecodingKey::from_ed_components(&body.x).context("invalid relay jwt public key")?;

        let mut guard = self.relay_jwt_decoding_key.write().await;
        *guard = Some(decoding_key);
        Ok(())
    }

    pub async fn run_relay_jwt_key_refresh_loop(self) {
        loop {
            match self.refresh_relay_jwt_decoding_key_once().await {
                Ok(()) => tokio::time::sleep(JWT_KEY_REFRESH_SUCCESS_INTERVAL).await,
                Err(err) => {
                    eprintln!(
                        "[{:?}] - relay jwt key refresh failed: {err}",
                        self.process_start.elapsed()
                    );
                    tokio::time::sleep(JWT_KEY_REFRESH_RETRY_INTERVAL).await;
                }
            }
        }
    }

    pub async fn verify_relay_jwt(&self, token: &str) -> Result<ConnectionIdentity> {
        let key = {
            let guard = self.relay_jwt_decoding_key.read().await;
            guard
                .clone()
                .ok_or_else(|| anyhow!("relay jwt verification key unavailable"))?
        };

        let mut validation = Validation::new(Algorithm::EdDSA);
        validation.required_spec_claims.insert("exp".to_string());

        let token_data = decode::<RelayJwtClaims>(token, &key, &validation)
            .map_err(|err| anyhow!("invalid relay token: {err}"))?;
        let claims = token_data.claims;

        if claims.sub.trim().is_empty() {
            return Err(anyhow!("invalid relay token: missing subject"));
        }

        if claims.exp <= 0 {
            return Err(anyhow!("invalid relay token: bad exp"));
        }

        let guid = Uuid::parse_str(&claims.sub)
            .map_err(|_| anyhow!("invalid relay token: bad subject guid"))?
            .to_string();

        Ok(ConnectionIdentity {
            guid,
            client_type: claims.client_type,
        })
    }
}
