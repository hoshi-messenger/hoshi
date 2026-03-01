use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail};
use futures_util::{SinkExt, StreamExt};
use hoshi_protocol::{
    common::ErrorResponse,
    control_plane::{
        ClientType, IssueRelayTokenRequest, IssueRelayTokenResponse, LookupClientResponse,
        NoisePublicKeyResponse, RelayEntry, RelayTokenProofPayload,
    },
    noise::{
        E2EE_NOISE_PATTERN, create_registration_handshake, create_relay_initiator_handshake,
        decode_base64, decode_base64_32, decrypt_e2ee_payload, derive_public_key, encode_base64,
        encrypt_e2ee_payload, finish_relay_initiator_handshake,
    },
    relay::{E2eeEnvelope, RelayErrorPacket, RelayPacket},
};
use rand_core::{OsRng, RngCore};
use reqwest::StatusCode;
use tokio::net::TcpStream;
use tokio_tungstenite::{
    MaybeTlsStream, WebSocketStream, connect_async,
    tungstenite::{
        Message,
        client::IntoClientRequest,
        http::{HeaderValue, header},
    },
};
use uuid::Uuid;

use crate::ClientDatabase;

const REQUEST_TIMEOUT: Duration = Duration::from_secs(3);

#[derive(Debug, Clone)]
pub struct ReceivedMessage {
    pub recipient: String,
    pub payload: Vec<u8>,
}

#[derive(Debug, Clone)]
pub struct ConnectConfiguredReport {
    pub connected_indices: Vec<usize>,
    pub errors: Vec<ConnectConfiguredError>,
}

#[derive(Debug, Clone)]
pub struct ConnectConfiguredError {
    pub config_key: String,
    pub guid: String,
    pub client_type: ClientType,
    pub error: String,
}

pub struct ClientManager {
    db: ClientDatabase,
    http_client: reqwest::Client,
    connections: Vec<ClientConnection>,
}

impl ClientManager {
    pub async fn new_default() -> Result<Self> {
        let db = ClientDatabase::open_default().await?;
        Self::new(db).await
    }

    pub async fn new(db: ClientDatabase) -> Result<Self> {
        let http_client = build_http_client()?;
        Ok(Self {
            db,
            http_client,
            connections: Vec::new(),
        })
    }

    pub async fn connect(&mut self, guid: &str, private_key_b64: &str) -> Result<usize> {
        self.connect_internal(guid, private_key_b64, None).await
    }

    pub async fn connect_from_store(
        &mut self,
        guid: &str,
        client_type: ClientType,
    ) -> Result<usize> {
        let key = self
            .db
            .get_key(guid, client_type.clone())
            .await?
            .ok_or_else(|| anyhow!("missing local key for guid {} ({:?})", guid, client_type))?;

        self.connect_internal(&key.guid, &key.private_key, Some(client_type))
            .await
    }

    pub async fn connect_configured(&mut self) -> Result<ConnectConfiguredReport> {
        let mut report = ConnectConfiguredReport {
            connected_indices: Vec::new(),
            errors: Vec::new(),
        };

        let configured = vec![
            (
                "device_guid".to_string(),
                self.db.get_device_guid().await?,
                ClientType::Device,
            ),
            (
                "user_guid".to_string(),
                self.db.get_user_guid().await?,
                ClientType::User,
            ),
        ];

        for (config_key, guid, client_type) in configured {
            let Some(guid) = guid else {
                continue;
            };

            match self.connect_from_store(&guid, client_type.clone()).await {
                Ok(index) => report.connected_indices.push(index),
                Err(err) => report.errors.push(ConnectConfiguredError {
                    config_key,
                    guid,
                    client_type,
                    error: err.to_string(),
                }),
            }
        }

        Ok(report)
    }

    pub fn connections(&self) -> &[ClientConnection] {
        &self.connections
    }

    pub fn connection(&self, index: usize) -> Option<&ClientConnection> {
        self.connections.get(index)
    }

    pub fn connection_mut(&mut self, index: usize) -> Option<&mut ClientConnection> {
        self.connections.get_mut(index)
    }

    pub async fn close_all(&mut self) -> Result<()> {
        let mut first_error: Option<anyhow::Error> = None;

        for connection in self.connections.iter_mut() {
            if let Err(err) = connection.close().await
                && first_error.is_none()
            {
                first_error = Some(err);
            }
        }

        self.connections.clear();

        if let Some(err) = first_error {
            return Err(err);
        }

        Ok(())
    }

    async fn connect_internal(
        &mut self,
        guid: &str,
        private_key_b64: &str,
        expected_client_type: Option<ClientType>,
    ) -> Result<usize> {
        let guid = canonicalize_guid(guid)?;
        let local_private_key = decode_base64_32(private_key_b64.trim(), "private_key")?;
        let control_plane_uri = self.db.get_control_plane_uri().await?;
        let control_plane_uri = control_plane_uri.trim_end_matches('/').to_string();

        let cp_noise = fetch_control_plane_noise_key(&self.http_client, &control_plane_uri).await?;
        let mut relays = list_relays(&self.http_client, &control_plane_uri).await?;
        shuffle_relays(&mut relays);

        let local_public_key = encode_base64(&derive_public_key(&local_private_key));
        let token = match issue_relay_token(
            &self.http_client,
            &control_plane_uri,
            &local_private_key,
            &local_public_key,
            &cp_noise.public_key,
        )
        .await
        {
            Ok(token) => token,
            Err(RelayTokenIssueError::UnknownClient) => {
                bail!("unknown client identity for guid {}", guid)
            }
            Err(RelayTokenIssueError::Other(err)) => {
                return Err(err.context("failed to issue relay token"));
            }
        };

        if token.guid != guid {
            bail!(
                "relay token guid mismatch: requested {}, got {}",
                guid,
                token.guid
            );
        }

        if let Some(expected_client_type) = expected_client_type {
            ensure_token_identity_type(&token, expected_client_type)?;
        }

        let mut connect_errors = Vec::new();
        let mut connected: Option<ClientConnection> = None;

        for relay in relays {
            match connect_authenticated_identity(
                self.http_client.clone(),
                &control_plane_uri,
                &relay,
                local_private_key,
                &token,
            )
            .await
            {
                Ok(connection) => {
                    connected = Some(connection);
                    break;
                }
                Err(err) => connect_errors.push(format!("{}:{} ({err})", relay.ip, relay.port)),
            }
        }

        let connection = match connected {
            Some(connection) => connection,
            None => bail!(
                "failed to connect to any relay: {}",
                connect_errors.join("; ")
            ),
        };

        self.db
            .upsert_key(&guid, token.client_type.clone(), private_key_b64)
            .await?;

        let index = self.connections.len();
        self.connections.push(connection);
        Ok(index)
    }
}

pub struct ClientConnection {
    control_plane_uri: String,
    http_client: reqwest::Client,
    websocket: WebSocketStream<MaybeTlsStream<TcpStream>>,
    relay_transport: snow::TransportState,
    local_private_key: [u8; 32],
    guid: String,
    client_type: ClientType,
    relay_guid: String,
}

impl ClientConnection {
    pub async fn send_message(&mut self, recipient_guid: &str, payload: &[u8]) -> Result<()> {
        let recipient_guid = recipient_guid.trim();
        if recipient_guid.is_empty() {
            bail!("recipient guid must not be empty");
        }

        let cp_uri = self.control_plane_uri.trim_end_matches('/');
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

    pub fn guid(&self) -> &str {
        &self.guid
    }

    pub fn client_type(&self) -> ClientType {
        self.client_type.clone()
    }

    pub fn relay_guid(&self) -> &str {
        &self.relay_guid
    }
}

#[derive(Debug)]
enum RelayTokenIssueError {
    UnknownClient,
    Other(anyhow::Error),
}

fn build_http_client() -> Result<reqwest::Client> {
    reqwest::Client::builder()
        .timeout(REQUEST_TIMEOUT)
        .build()
        .context("failed to build client http client")
}

fn canonicalize_guid(guid: &str) -> Result<String> {
    Ok(Uuid::parse_str(guid.trim())
        .context("invalid guid")?
        .to_string())
}

fn ensure_token_identity_type(
    token: &IssueRelayTokenResponse,
    expected_client_type: ClientType,
) -> Result<()> {
    if token.client_type != expected_client_type {
        bail!(
            "relay token identity type mismatch: expected {:?}, got {:?}",
            expected_client_type,
            token.client_type
        );
    }
    Ok(())
}

async fn connect_authenticated_identity(
    http_client: reqwest::Client,
    control_plane_uri: &str,
    relay: &RelayEntry,
    local_private_key: [u8; 32],
    token: &IssueRelayTokenResponse,
) -> Result<ClientConnection> {
    let (mut websocket, relay_transport) = connect_relay(relay, &token.token, &token.guid).await?;
    let _ = websocket.flush().await;

    Ok(ClientConnection {
        control_plane_uri: control_plane_uri.to_string(),
        http_client,
        websocket,
        relay_transport,
        local_private_key,
        guid: token.guid.clone(),
        client_type: token.client_type.clone(),
        relay_guid: relay.guid.clone(),
    })
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
) -> std::result::Result<IssueRelayTokenResponse, RelayTokenIssueError> {
    let endpoint = format!("{control_plane_uri}/auth/relay-token");
    let proof_payload = serde_json::to_vec(&RelayTokenProofPayload {
        public_key: noise_public_key.to_string(),
    })
    .map_err(|err| RelayTokenIssueError::Other(anyhow!(err)))?;
    let noise_handshake =
        create_registration_handshake(noise_private_key, cp_noise_public_key, &proof_payload)
            .map_err(RelayTokenIssueError::Other)?;

    let response = http_client
        .post(&endpoint)
        .json(&IssueRelayTokenRequest {
            public_key: noise_public_key.to_string(),
            noise_handshake: encode_base64(&noise_handshake),
        })
        .send()
        .await
        .with_context(|| format!("failed to issue relay token: {endpoint}"))
        .map_err(RelayTokenIssueError::Other)?;
    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        if status == StatusCode::UNAUTHORIZED
            && let Ok(parsed) = serde_json::from_str::<ErrorResponse>(&body)
            && parsed.error == "unknown client"
        {
            return Err(RelayTokenIssueError::UnknownClient);
        }

        if let Ok(parsed) = serde_json::from_str::<ErrorResponse>(&body) {
            return Err(RelayTokenIssueError::Other(anyhow!(
                "relay token request failed: {status} ({})",
                parsed.error
            )));
        }
        if body.trim().is_empty() {
            return Err(RelayTokenIssueError::Other(anyhow!(
                "relay token request failed: {status}"
            )));
        }
        return Err(RelayTokenIssueError::Other(anyhow!(
            "relay token request failed: {status} ({body})"
        )));
    }

    response
        .json::<IssueRelayTokenResponse>()
        .await
        .context("failed to decode relay token payload")
        .map_err(RelayTokenIssueError::Other)
}

async fn list_relays(
    http_client: &reqwest::Client,
    control_plane_uri: &str,
) -> Result<Vec<RelayEntry>> {
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

    Ok(relays)
}

fn shuffle_relays(relays: &mut [RelayEntry]) {
    for i in (1..relays.len()).rev() {
        let j = (OsRng.next_u64() as usize) % (i + 1);
        relays.swap(i, j);
    }
}

async fn connect_relay(
    relay: &RelayEntry,
    token: &str,
    guid: &str,
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
        .with_context(|| format!("failed to build relay noise handshake for guid {guid}"))?;

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
