use anyhow::{Context, Result, anyhow};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use serde::Serialize;
use x25519_dalek::{X25519_BASEPOINT_BYTES, x25519};

pub const REGISTRATION_NOISE_PATTERN: &str = "Noise_X_25519_ChaChaPoly_BLAKE2s";
pub const RELAY_SESSION_NOISE_PATTERN: &str = "Noise_NK_25519_ChaChaPoly_BLAKE2s";
pub const E2EE_NOISE_PATTERN: &str = "Noise_N_25519_ChaChaPoly_BLAKE2s";

const MAX_NOISE_MESSAGE_LEN: usize = 65_535;

fn parse_noise_params(pattern: &str) -> Result<snow::params::NoiseParams> {
    pattern
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

pub fn generate_static_private_key() -> Result<[u8; 32]> {
    let keypair = snow::Builder::new(parse_noise_params(REGISTRATION_NOISE_PATTERN)?)
        .generate_keypair()
        .context("failed to generate noise static keypair")?;

    keypair
        .private
        .as_slice()
        .try_into()
        .map_err(|_| anyhow!("generated noise private key has invalid length"))
}

pub fn serialize_payload<T: Serialize>(payload: &T) -> Result<Vec<u8>> {
    serde_json::to_vec(payload).context("failed to serialize payload")
}

pub fn create_registration_handshake(
    local_private_key: &[u8; 32],
    remote_public_key_b64: &str,
    payload: &[u8],
) -> Result<Vec<u8>> {
    let remote_public_key = decode_base64_32(remote_public_key_b64, "public_key")?;
    let mut initiator = snow::Builder::new(parse_noise_params(REGISTRATION_NOISE_PATTERN)?)
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

pub fn verify_registration_proof(
    server_private_key: &[u8; 32],
    claimed_public_key: &[u8; 32],
    handshake_message: &[u8],
    expected_payload: &[u8],
) -> Result<()> {
    if handshake_message.len() > MAX_NOISE_MESSAGE_LEN {
        return Err(anyhow!("handshake message too large"));
    }

    let mut responder = snow::Builder::new(parse_noise_params(REGISTRATION_NOISE_PATTERN)?)
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

pub fn create_relay_initiator_handshake(
    relay_public_key_b64: &str,
) -> Result<(snow::HandshakeState, Vec<u8>)> {
    let relay_public_key = decode_base64_32(relay_public_key_b64, "public_key")?;
    let mut initiator = snow::Builder::new(parse_noise_params(RELAY_SESSION_NOISE_PATTERN)?)
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

pub fn accept_relay_responder_handshake(
    local_private_key: &[u8; 32],
    message: &[u8],
) -> Result<(snow::TransportState, Vec<u8>)> {
    let mut responder = snow::Builder::new(parse_noise_params(RELAY_SESSION_NOISE_PATTERN)?)
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

pub fn encrypt_e2ee_payload(recipient_public_key_b64: &str, payload: &[u8]) -> Result<Vec<u8>> {
    let recipient_public_key = decode_base64_32(recipient_public_key_b64, "recipient_public_key")?;
    let mut initiator = snow::Builder::new(parse_noise_params(E2EE_NOISE_PATTERN)?)
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
    let mut responder = snow::Builder::new(parse_noise_params(E2EE_NOISE_PATTERN)?)
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
    use super::{
        accept_relay_responder_handshake, create_registration_handshake,
        create_relay_initiator_handshake, decrypt_e2ee_payload, derive_public_key, encode_base64,
        encrypt_e2ee_payload, finish_relay_initiator_handshake, generate_static_private_key,
        serialize_payload, verify_registration_proof,
    };
    use crate::control_plane::{ClientRegistrationProofPayload, ClientType};

    #[test]
    fn e2ee_encrypt_decrypt_roundtrip() {
        let recipient_private = [7_u8; 32];
        let recipient_public = encode_base64(&derive_public_key(&recipient_private));
        let payload = b"hello-e2ee";

        let ciphertext = encrypt_e2ee_payload(&recipient_public, payload).expect("encrypt");
        let decrypted = decrypt_e2ee_payload(&recipient_private, &ciphertext).expect("decrypt");

        assert_eq!(decrypted, payload);
    }

    #[test]
    fn registration_handshake_roundtrip() {
        let server_private = generate_static_private_key().expect("server key");
        let client_private = generate_static_private_key().expect("client key");
        let client_public = derive_public_key(&client_private);
        let server_public_b64 = encode_base64(&derive_public_key(&server_private));

        let payload = serialize_payload(&ClientRegistrationProofPayload {
            public_key: encode_base64(&client_public),
            client_type: ClientType::Device,
        })
        .expect("payload");
        let handshake =
            create_registration_handshake(&client_private, &server_public_b64, &payload)
                .expect("handshake");

        verify_registration_proof(&server_private, &client_public, &handshake, &payload)
            .expect("verify");
    }

    #[test]
    fn relay_session_handshake_roundtrip() {
        let relay_private = generate_static_private_key().expect("relay key");
        let relay_public_b64 = encode_base64(&derive_public_key(&relay_private));

        let (initiator, handshake) =
            create_relay_initiator_handshake(&relay_public_b64).expect("initiator");
        let (responder_transport, response) =
            accept_relay_responder_handshake(&relay_private, &handshake).expect("responder");
        let initiator_transport =
            finish_relay_initiator_handshake(initiator, &response).expect("finish");

        let mut outbound = vec![0_u8; 128];
        let mut inbound = vec![0_u8; 128];
        let mut initiator_transport = initiator_transport;
        let mut responder_transport = responder_transport;

        let written = initiator_transport
            .write_message(b"ping", &mut outbound)
            .expect("encrypt");
        let read = responder_transport
            .read_message(&outbound[..written], &mut inbound)
            .expect("decrypt");

        assert_eq!(&inbound[..read], b"ping");
    }
}
