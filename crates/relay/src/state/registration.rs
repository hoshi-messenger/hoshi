use std::time::Duration;

use anyhow::{Context, Result, anyhow};
use hoshi_protocol::control_plane::{
    NoisePublicKeyResponse, RegisterRelayRequest, RelayRegistrationProofPayload,
};
use hoshi_protocol::noise::{create_registration_handshake, encode_base64, serialize_payload};

use super::ServerState;

const RELAY_REGISTRATION_SUCCESS_INTERVAL: Duration = Duration::from_secs(60);
const RELAY_REGISTRATION_RETRY_INTERVAL: Duration = Duration::from_secs(10);

impl ServerState {
    pub async fn run_relay_registration_loop(self) {
        loop {
            match self.register_with_control_plane_once().await {
                Ok(()) => tokio::time::sleep(RELAY_REGISTRATION_SUCCESS_INTERVAL).await,
                Err(err) => {
                    eprintln!(
                        "[{:?}] - relay registration refresh failed: {err}",
                        self.process_start.elapsed()
                    );
                    tokio::time::sleep(RELAY_REGISTRATION_RETRY_INTERVAL).await;
                }
            }
        }
    }

    pub async fn register_with_control_plane_once(&self) -> Result<()> {
        let cp_uri = self.config.control_plane_uri.trim_end_matches('/');
        let noise_endpoint = format!("{cp_uri}/noise/public-key");
        let register_endpoint = format!("{cp_uri}/relays");

        let noise_response = self
            .http_client
            .get(&noise_endpoint)
            .send()
            .await
            .with_context(|| {
                format!("failed to fetch control-plane noise key: {noise_endpoint}")
            })?;

        if !noise_response.status().is_success() {
            return Err(anyhow!(
                "control-plane noise key returned non-success status: {}",
                noise_response.status()
            ));
        }

        let noise_payload = noise_response
            .json::<NoisePublicKeyResponse>()
            .await
            .context("failed to decode control-plane noise key payload")?;

        let proof_payload = RelayRegistrationProofPayload {
            public_key: self.noise_public_key().to_string(),
            guid: self.config.guid.clone(),
            api_key: self.config.api_key.clone(),
            port: self.config.http_bind_address.port(),
        };
        let proof_payload = serialize_payload(&proof_payload)?;
        let handshake = create_registration_handshake(
            self.noise_static_private_key(),
            &noise_payload.public_key,
            &proof_payload,
        )?;

        let request = RegisterRelayRequest {
            public_key: self.noise_public_key().to_string(),
            guid: self.config.guid.clone(),
            api_key: self.config.api_key.clone(),
            port: self.config.http_bind_address.port(),
            noise_handshake: encode_base64(&handshake),
        };

        let response = self
            .http_client
            .post(&register_endpoint)
            .json(&request)
            .send()
            .await
            .with_context(|| {
                format!(
                    "failed to register relay with control-plane: {}",
                    register_endpoint
                )
            })?;

        if response.status().is_success() {
            Ok(())
        } else {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            Err(anyhow!(
                "relay registration failed with status {status}: {body}"
            ))
        }
    }
}
