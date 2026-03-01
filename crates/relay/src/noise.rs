use anyhow::{Context, Result, anyhow};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use serde::Serialize;
use x25519_dalek::{X25519_BASEPOINT_BYTES, x25519};

pub const REGISTRATION_NOISE_PATTERN: &str = "Noise_X_25519_ChaChaPoly_BLAKE2s";
pub const RELAY_SESSION_NOISE_PATTERN: &str = "Noise_NK_25519_ChaChaPoly_BLAKE2s";

pub fn parse_registration_noise_params() -> Result<snow::params::NoiseParams> {
    REGISTRATION_NOISE_PATTERN
        .parse()
        .map_err(|err| anyhow!("invalid noise pattern: {err}"))
}

pub fn parse_relay_session_noise_params() -> Result<snow::params::NoiseParams> {
    RELAY_SESSION_NOISE_PATTERN
        .parse()
        .map_err(|err| anyhow!("invalid noise pattern: {err}"))
}

pub fn encode_base64(value: &[u8]) -> String {
    STANDARD.encode(value)
}

pub fn decode_base64(value: &str) -> Result<Vec<u8>> {
    STANDARD
        .decode(value)
        .map_err(|err| anyhow!("invalid base64: {err}"))
}

pub fn decode_base64_32(value: &str, field: &str) -> Result<[u8; 32]> {
    let decoded = decode_base64(value).map_err(|_| anyhow!("invalid {field} base64"))?;
    decoded
        .try_into()
        .map_err(|_| anyhow!("invalid {field} length"))
}

pub fn canonicalize_base64_32(value: &str, field: &str) -> Result<(String, [u8; 32])> {
    let decoded = decode_base64_32(value, field)?;
    Ok((encode_base64(&decoded), decoded))
}

pub fn derive_public_key(private_key: &[u8; 32]) -> [u8; 32] {
    x25519(*private_key, X25519_BASEPOINT_BYTES)
}

pub fn serialize_payload<T: Serialize>(payload: &T) -> Result<Vec<u8>> {
    serde_json::to_vec(payload).context("failed to serialize payload")
}

pub fn create_initiator_handshake(
    local_private_key: &[u8; 32],
    remote_public_key_b64: &str,
    payload: &[u8],
) -> Result<Vec<u8>> {
    let remote_public_key = decode_base64_32(remote_public_key_b64, "public_key")?;
    let mut initiator = snow::Builder::new(parse_registration_noise_params()?)
        .local_private_key(local_private_key)
        .remote_public_key(&remote_public_key)
        .build_initiator()
        .map_err(|_| anyhow!("failed to build noise initiator"))?;

    let mut message = vec![0_u8; payload.len() + 256];
    let message_len = initiator
        .write_message(payload, &mut message)
        .map_err(|_| anyhow!("failed to write noise handshake"))?;
    Ok(message[..message_len].to_vec())
}

pub fn accept_responder_handshake(
    local_private_key: &[u8; 32],
    message: &[u8],
) -> Result<(snow::TransportState, Vec<u8>)> {
    let mut responder = snow::Builder::new(parse_relay_session_noise_params()?)
        .local_private_key(local_private_key)
        .build_responder()
        .map_err(|_| anyhow!("failed to build noise responder"))?;

    let mut payload = vec![0_u8; message.len().max(1)];
    responder
        .read_message(message, &mut payload)
        .map_err(|_| anyhow!("invalid noise handshake"))?;

    let mut response = vec![0_u8; 256];
    let response_len = responder
        .write_message(&[], &mut response)
        .map_err(|_| anyhow!("invalid noise handshake"))?;

    if !responder.is_handshake_finished() {
        return Err(anyhow!("invalid noise handshake"));
    }

    let transport = responder
        .into_transport_mode()
        .map_err(|_| anyhow!("failed to switch noise transport mode"))?;
    Ok((transport, response[..response_len].to_vec()))
}
