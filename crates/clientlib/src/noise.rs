use anyhow::{Result, anyhow};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use x25519_dalek::{X25519_BASEPOINT_BYTES, x25519};

pub const REGISTRATION_NOISE_PATTERN: &str = "Noise_X_25519_ChaChaPoly_BLAKE2s";
pub const RELAY_SESSION_NOISE_PATTERN: &str = "Noise_NK_25519_ChaChaPoly_BLAKE2s";
pub const E2EE_NOISE_PATTERN: &str = "Noise_N_25519_ChaChaPoly_BLAKE2s";

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

pub fn canonicalize_base64_32(value: &str, field: &str) -> Result<String> {
    let decoded = decode_base64_32(value, field)?;
    Ok(encode_base64(&decoded))
}

pub fn derive_public_key(private_key: &[u8; 32]) -> [u8; 32] {
    x25519(*private_key, X25519_BASEPOINT_BYTES)
}

pub fn create_registration_handshake(
    local_private_key: &[u8; 32],
    remote_public_key_b64: &str,
    payload: &[u8],
) -> Result<Vec<u8>> {
    let remote_public_key = decode_base64_32(remote_public_key_b64, "public_key")?;
    let params = REGISTRATION_NOISE_PATTERN
        .parse()
        .map_err(|err| anyhow!("invalid noise pattern: {err}"))?;
    let mut initiator = snow::Builder::new(params)
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

pub fn create_relay_initiator_handshake(
    relay_public_key_b64: &str,
) -> Result<(snow::HandshakeState, Vec<u8>)> {
    let relay_public_key = decode_base64_32(relay_public_key_b64, "public_key")?;
    let params = RELAY_SESSION_NOISE_PATTERN
        .parse()
        .map_err(|err| anyhow!("invalid noise pattern: {err}"))?;
    let mut initiator = snow::Builder::new(params)
        .remote_public_key(&relay_public_key)
        .build_initiator()
        .map_err(|_| anyhow!("failed to build relay initiator"))?;

    let mut handshake = vec![0_u8; 256];
    let handshake_len = initiator
        .write_message(&[], &mut handshake)
        .map_err(|_| anyhow!("failed to write relay handshake"))?;

    Ok((initiator, handshake[..handshake_len].to_vec()))
}

pub fn finish_relay_initiator_handshake(
    mut initiator: snow::HandshakeState,
    response_message: &[u8],
) -> Result<snow::TransportState> {
    let mut payload = vec![0_u8; response_message.len().max(1)];
    initiator
        .read_message(response_message, &mut payload)
        .map_err(|_| anyhow!("invalid relay handshake response"))?;

    if !initiator.is_handshake_finished() {
        return Err(anyhow!("relay handshake did not finish"));
    }

    initiator
        .into_transport_mode()
        .map_err(|_| anyhow!("failed to switch relay transport mode"))
}

pub fn encrypt_e2ee_payload(recipient_public_key_b64: &str, payload: &[u8]) -> Result<Vec<u8>> {
    let recipient_public_key = decode_base64_32(recipient_public_key_b64, "recipient_public_key")?;
    let params = E2EE_NOISE_PATTERN
        .parse()
        .map_err(|err| anyhow!("invalid noise pattern: {err}"))?;
    let mut initiator = snow::Builder::new(params)
        .remote_public_key(&recipient_public_key)
        .build_initiator()
        .map_err(|_| anyhow!("failed to build e2ee initiator"))?;

    let mut ciphertext = vec![0_u8; payload.len() + 256];
    let ciphertext_len = initiator
        .write_message(payload, &mut ciphertext)
        .map_err(|_| anyhow!("failed to encrypt e2ee payload"))?;
    Ok(ciphertext[..ciphertext_len].to_vec())
}

pub fn decrypt_e2ee_payload(local_private_key: &[u8; 32], message: &[u8]) -> Result<Vec<u8>> {
    let params = E2EE_NOISE_PATTERN
        .parse()
        .map_err(|err| anyhow!("invalid noise pattern: {err}"))?;
    let mut responder = snow::Builder::new(params)
        .local_private_key(local_private_key)
        .build_responder()
        .map_err(|_| anyhow!("failed to build e2ee responder"))?;

    let mut plaintext = vec![0_u8; message.len().max(1)];
    let plaintext_len = responder
        .read_message(message, &mut plaintext)
        .map_err(|_| anyhow!("failed to decrypt e2ee payload"))?;

    if !responder.is_handshake_finished() {
        return Err(anyhow!("invalid e2ee packet"));
    }

    Ok(plaintext[..plaintext_len].to_vec())
}

#[cfg(test)]
mod tests {
    use super::{decrypt_e2ee_payload, derive_public_key, encode_base64, encrypt_e2ee_payload};

    #[test]
    fn e2ee_encrypt_decrypt_roundtrip() {
        let recipient_private = [7_u8; 32];
        let recipient_public = encode_base64(&derive_public_key(&recipient_private));
        let payload = b"hello-e2ee";

        let ciphertext = encrypt_e2ee_payload(&recipient_public, payload).expect("encrypt");
        let decrypted = decrypt_e2ee_payload(&recipient_private, &ciphertext).expect("decrypt");

        assert_eq!(decrypted, payload);
    }
}
