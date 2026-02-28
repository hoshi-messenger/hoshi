use anyhow::{Context, Result, anyhow};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use serde::Serialize;
use x25519_dalek::{X25519_BASEPOINT_BYTES, x25519};

pub const REGISTRATION_NOISE_PATTERN: &str = "Noise_X_25519_ChaChaPoly_BLAKE2s";
const MAX_NOISE_MESSAGE_LEN: usize = 65_535;

pub fn parse_noise_params() -> Result<snow::params::NoiseParams> {
    REGISTRATION_NOISE_PATTERN
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

pub fn generate_static_private_key() -> Result<[u8; 32]> {
    let keypair = snow::Builder::new(parse_noise_params()?)
        .generate_keypair()
        .context("failed to generate noise static keypair")?;
    keypair
        .private
        .as_slice()
        .try_into()
        .map_err(|_| anyhow!("generated noise private key has invalid length"))
}

pub fn derive_public_key(private_key: &[u8; 32]) -> [u8; 32] {
    x25519(*private_key, X25519_BASEPOINT_BYTES)
}

pub fn serialize_proof_payload<T: Serialize>(payload: &T) -> Result<Vec<u8>> {
    serde_json::to_vec(payload).context("failed to serialize proof payload")
}

pub fn verify_registration_proof(
    server_private_key: &[u8; 32],
    claimed_public_key: &[u8; 32],
    handshake_message: &[u8],
    expected_payload: &[u8],
) -> Result<()> {
    if handshake_message.len() > MAX_NOISE_MESSAGE_LEN {
        return Err(anyhow!("handshake message too large"));
    }

    let mut responder = snow::Builder::new(parse_noise_params()?)
        .local_private_key(server_private_key)
        .build_responder()
        .map_err(|_| anyhow!("invalid registration proof"))?;

    let out_len = handshake_message.len().max(expected_payload.len()).max(1);
    let mut payload = vec![0_u8; out_len];
    let payload_len = responder
        .read_message(handshake_message, &mut payload)
        .map_err(|_| anyhow!("invalid registration proof"))?;

    if !responder.is_handshake_finished() {
        return Err(anyhow!("invalid registration proof"));
    }

    let Some(remote_static) = responder.get_remote_static() else {
        return Err(anyhow!("invalid registration proof"));
    };

    if remote_static != claimed_public_key {
        return Err(anyhow!("invalid registration proof"));
    }

    if payload_len != expected_payload.len() {
        return Err(anyhow!("invalid registration proof"));
    }

    if payload[..payload_len] != expected_payload[..] {
        return Err(anyhow!("invalid registration proof"));
    }

    Ok(())
}
