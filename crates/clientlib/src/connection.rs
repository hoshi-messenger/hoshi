use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail};
use futures_util::{SinkExt, StreamExt};
use hoshi_protocol::{
    common::ErrorResponse,
    control_plane::{
        IssueRelayTokenRequest, IssueRelayTokenResponse, LookupClientResponse,
        NoisePublicKeyResponse, RelayEntry,
    },
    relay::{RelayErrorPacket, RelayPacket},
};
use rand_core::{OsRng, RngCore};
use serde::Serialize;
use tokio::net::TcpStream;
use tokio_tungstenite::{
    MaybeTlsStream, WebSocketStream, connect_async,
    tungstenite::{
        Message,
        client::IntoClientRequest,
        http::{HeaderValue, header},
    },
};

use crate::{
    ClientConfig,
    noise::{
        E2EE_NOISE_PATTERN, create_registration_handshake, create_relay_initiator_handshake,
        decode_base64, decrypt_e2ee_payload, derive_public_key, encode_base64,
        encrypt_e2ee_payload, finish_relay_initiator_handshake,
    },
};

const REQUEST_TIMEOUT: Duration = Duration::from_secs(3);

#[derive(Debug, Serialize)]
struct RelayTokenProofPayload<'a> {
    public_key: &'a str,
}

#[derive(Debug, Clone, Serialize, serde::Deserialize)]
struct E2eeEnvelope {
    version: u8,
    alg: String,
    ciphertext: String,
}

#[derive(Debug, Clone)]
pub struct ReceivedMessage {
    pub recipient: String,
    pub payload: Vec<u8>,
}

pub struct ClientConnection {
    config: ClientConfig,
    http_client: reqwest::Client,
    websocket: WebSocketStream<MaybeTlsStream<TcpStream>>,
    relay_transport: snow::TransportState,
    local_private_key: [u8; 32],
    client_guid: String,
    device_guid: String,
    relay_guid: String,
}

impl ClientConnection {
    pub async fn connect() -> Result<Self> {
        let config = ClientConfig::new()?;
        Self::connect_with_config(config).await
    }

    pub async fn connect_with_config(config: ClientConfig) -> Result<Self> {
        let http_client = reqwest::Client::builder()
            .timeout(REQUEST_TIMEOUT)
            .build()
            .context("failed to build client http client")?;

        let noise_private_key = config.noise_static_private_key_bytes()?;
        let noise_public_key = encode_base64(&derive_public_key(&noise_private_key));
        let cp_uri = config.control_plane_uri.trim_end_matches('/');

        let cp_noise = fetch_control_plane_noise_key(&http_client, cp_uri).await?;
        let token = issue_relay_token(
            &http_client,
            cp_uri,
            &noise_private_key,
            &noise_public_key,
            &cp_noise.public_key,
        )
        .await?;
        let relay = select_relay(&http_client, cp_uri).await?;
        let (mut websocket, relay_transport) =
            connect_relay(&relay, &token.token, &token.device_guid).await?;

        let _ = websocket.flush().await;

        Ok(Self {
            config,
            http_client,
            websocket,
            relay_transport,
            local_private_key: noise_private_key,
            client_guid: token.client_guid,
            device_guid: token.device_guid,
            relay_guid: relay.guid,
        })
    }

    pub async fn send_message(&mut self, recipient_guid: &str, payload: &[u8]) -> Result<()> {
        let recipient_guid = recipient_guid.trim();
        if recipient_guid.is_empty() {
            bail!("recipient guid must not be empty");
        }

        let cp_uri = self.config.control_plane_uri.trim_end_matches('/');
        let lookup_endpoint = format!("{cp_uri}/clients/{recipient_guid}");
        let lookup_response = self
            .http_client
            .get(&lookup_endpoint)
            .send()
            .await
            .with_context(|| format!("failed to lookup recipient client: {lookup_endpoint}"))?;

        if !lookup_response.status().is_success() {
            return Err(response_error(lookup_response, "recipient lookup failed").await);
        }

        let lookup = lookup_response
            .json::<LookupClientResponse>()
            .await
            .context("failed to decode recipient lookup payload")?;
        let e2ee_ciphertext = encrypt_e2ee_payload(&lookup.public_key, payload)?;
        let envelope = E2eeEnvelope {
            version: 1,
            alg: E2EE_NOISE_PATTERN.to_string(),
            ciphertext: encode_base64(&e2ee_ciphertext),
        };
        let relay_packet = RelayPacket {
            recipient: recipient_guid.to_string(),
            payload: serde_json::to_string(&envelope)
                .context("failed to serialize e2ee envelope")?,
        };

        let relay_payload =
            serde_json::to_vec(&relay_packet).context("failed to serialize relay packet")?;
        let mut relay_ciphertext = vec![0_u8; relay_payload.len() + 1024];
        let relay_ciphertext_len = self
            .relay_transport
            .write_message(&relay_payload, &mut relay_ciphertext)
            .map_err(|_| anyhow!("failed to encrypt relay payload"))?;

        self.websocket
            .send(Message::Binary(
                relay_ciphertext[..relay_ciphertext_len].to_vec().into(),
            ))
            .await
            .context("failed to write message to relay websocket")?;

        Ok(())
    }

    pub async fn send_text(&mut self, recipient_guid: &str, text: &str) -> Result<()> {
        self.send_message(recipient_guid, text.as_bytes()).await
    }

    pub async fn receive_message(&mut self) -> Result<ReceivedMessage> {
        loop {
            let Some(frame) = self.websocket.next().await else {
                bail!("relay websocket closed");
            };
            let frame = frame.context("failed to receive relay websocket frame")?;

            match frame {
                Message::Binary(ciphertext) => {
                    let mut relay_plaintext = vec![0_u8; ciphertext.len().max(1) + 1024];
                    let relay_plaintext_len = self
                        .relay_transport
                        .read_message(&ciphertext, &mut relay_plaintext)
                        .map_err(|_| anyhow!("failed to decrypt relay packet"))?;
                    let relay_plaintext = &relay_plaintext[..relay_plaintext_len];

                    if let Ok(err) = serde_json::from_slice::<RelayErrorPacket>(relay_plaintext) {
                        if let Some(recipient) = err.recipient {
                            bail!("relay error: {} ({recipient})", err.error);
                        }
                        bail!("relay error: {}", err.error);
                    }

                    let relay_packet = serde_json::from_slice::<RelayPacket>(relay_plaintext)
                        .context("failed to decode relay packet")?;
                    let envelope = serde_json::from_str::<E2eeEnvelope>(&relay_packet.payload)
                        .context("failed to decode e2ee envelope")?;
                    if envelope.alg != E2EE_NOISE_PATTERN {
                        bail!("unsupported e2ee algorithm: {}", envelope.alg);
                    }

                    let e2ee_ciphertext = decode_base64(&envelope.ciphertext)
                        .context("failed to decode e2ee ciphertext")?;
                    let payload = decrypt_e2ee_payload(&self.local_private_key, &e2ee_ciphertext)?;

                    return Ok(ReceivedMessage {
                        recipient: relay_packet.recipient,
                        payload,
                    });
                }
                Message::Ping(_) | Message::Pong(_) => {}
                Message::Close(_) => bail!("relay websocket closed"),
                Message::Text(_) => bail!("unexpected text websocket frame from relay"),
                Message::Frame(_) => {}
            }
        }
    }

    pub async fn close(&mut self) -> Result<()> {
        self.websocket
            .close(None)
            .await
            .context("failed to close relay websocket")
    }

    pub fn client_guid(&self) -> &str {
        &self.client_guid
    }

    pub fn device_guid(&self) -> &str {
        &self.device_guid
    }

    pub fn relay_guid(&self) -> &str {
        &self.relay_guid
    }
}

async fn fetch_control_plane_noise_key(
    http_client: &reqwest::Client,
    control_plane_uri: &str,
) -> Result<NoisePublicKeyResponse> {
    let endpoint = format!("{control_plane_uri}/noise/public-key");
    let response = http_client
        .get(&endpoint)
        .send()
        .await
        .with_context(|| format!("failed to fetch control-plane noise key: {endpoint}"))?;
    if !response.status().is_success() {
        return Err(response_error(response, "control-plane noise key request failed").await);
    }

    response
        .json::<NoisePublicKeyResponse>()
        .await
        .context("failed to decode control-plane noise key payload")
}

async fn issue_relay_token(
    http_client: &reqwest::Client,
    control_plane_uri: &str,
    noise_private_key: &[u8; 32],
    noise_public_key: &str,
    cp_noise_public_key: &str,
) -> Result<IssueRelayTokenResponse> {
    let endpoint = format!("{control_plane_uri}/auth/relay-token");
    let proof_payload = serde_json::to_vec(&RelayTokenProofPayload {
        public_key: noise_public_key,
    })
    .context("failed to serialize relay token proof payload")?;
    let noise_handshake =
        create_registration_handshake(noise_private_key, cp_noise_public_key, &proof_payload)?;

    let response = http_client
        .post(&endpoint)
        .json(&IssueRelayTokenRequest {
            public_key: noise_public_key.to_string(),
            noise_handshake: encode_base64(&noise_handshake),
        })
        .send()
        .await
        .with_context(|| format!("failed to issue relay token: {endpoint}"))?;
    if !response.status().is_success() {
        return Err(response_error(response, "relay token request failed").await);
    }

    response
        .json::<IssueRelayTokenResponse>()
        .await
        .context("failed to decode relay token payload")
}

async fn select_relay(
    http_client: &reqwest::Client,
    control_plane_uri: &str,
) -> Result<RelayEntry> {
    let endpoint = format!("{control_plane_uri}/relays");
    let response = http_client
        .get(&endpoint)
        .send()
        .await
        .with_context(|| format!("failed to fetch relay registry: {endpoint}"))?;
    if !response.status().is_success() {
        return Err(response_error(response, "relay registry request failed").await);
    }

    let relays = response
        .json::<Vec<RelayEntry>>()
        .await
        .context("failed to decode relay registry payload")?;
    if relays.is_empty() {
        bail!("relay registry is empty");
    }

    let relay_index = (OsRng.next_u64() as usize) % relays.len();
    Ok(relays[relay_index].clone())
}

async fn connect_relay(
    relay: &RelayEntry,
    token: &str,
    device_guid: &str,
) -> Result<(
    WebSocketStream<MaybeTlsStream<TcpStream>>,
    snow::TransportState,
)> {
    let ws_uri = relay_ws_uri(relay);
    let mut request = ws_uri
        .clone()
        .into_client_request()
        .map_err(|err| anyhow!("failed to build relay websocket request: {err}"))?;
    request.headers_mut().insert(
        header::AUTHORIZATION,
        HeaderValue::from_str(&format!("Bearer {token}"))
            .map_err(|err| anyhow!("invalid authorization header: {err}"))?,
    );

    let (mut websocket, _response) = connect_async(request)
        .await
        .with_context(|| format!("failed to connect to relay websocket: {ws_uri}"))?;

    let (initiator, relay_handshake) = create_relay_initiator_handshake(&relay.public_key)
        .with_context(|| {
            format!("failed to build relay noise handshake for device {device_guid}")
        })?;

    websocket
        .send(Message::Binary(relay_handshake.into()))
        .await
        .context("failed to send relay noise handshake")?;

    let Some(response_frame) = websocket.next().await else {
        bail!("relay websocket closed during handshake");
    };
    let response_frame = response_frame.context("failed to receive relay handshake response")?;
    let response_message = match response_frame {
        Message::Binary(message) => message,
        Message::Close(_) => bail!("relay websocket closed during handshake"),
        _ => bail!("unexpected relay handshake frame"),
    };
    let relay_transport = finish_relay_initiator_handshake(initiator, &response_message)
        .context("failed to complete relay noise handshake")?;

    Ok((websocket, relay_transport))
}

fn relay_ws_uri(relay: &RelayEntry) -> String {
    let host = if relay.ip.contains(':') && !relay.ip.starts_with('[') {
        format!("[{}]", relay.ip)
    } else {
        relay.ip.clone()
    };
    format!("ws://{host}:{}/relay", relay.port)
}

async fn response_error(response: reqwest::Response, context: &str) -> anyhow::Error {
    let status = response.status();
    let body = response.text().await.unwrap_or_default();
    if let Ok(parsed) = serde_json::from_str::<ErrorResponse>(&body) {
        anyhow!("{context}: {status} ({})", parsed.error)
    } else if body.trim().is_empty() {
        anyhow!("{context}: {status}")
    } else {
        anyhow!("{context}: {status} ({body})")
    }
}
