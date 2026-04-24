use anyhow::{Result, anyhow};

fn hex_decode(s: &str) -> Option<Vec<u8>> {
    if s.len() % 2 != 0 {
        return None;
    }
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).ok())
        .collect()
}

fn hex_encode(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{:02x}", b)).collect()
}

pub fn user_path(public_key: &str) -> String {
    format!("/user/{public_key}")
}

pub fn normalize_public_key(public_key: &str) -> String {
    public_key.chars().filter(|c| !c.is_whitespace()).collect()
}

pub fn validate_public_key_hex(public_key: &str) -> Result<()> {
    let public_key = normalize_public_key(public_key);
    let Some(bytes) = hex_decode(&public_key) else {
        return Err(anyhow!("public key must be valid hex"));
    };
    if bytes.len() != 32 {
        return Err(anyhow!(
            "public key must be 32 bytes / 64 hex chars, got {} hex chars",
            public_key.len()
        ));
    }
    Ok(())
}

/// Compute the chat path for a 1:1 chat between two public keys.
/// Returns `/chat/{a_key XOR b_key}`.
pub fn chat_path(a: &str, b: &str) -> String {
    let a_bytes = hex_decode(a).expect("invalid hex public key");
    let b_bytes = hex_decode(b).expect("invalid hex public key");
    assert_eq!(a_bytes.len(), b_bytes.len(), "public key length mismatch");
    let xor: Vec<u8> = a_bytes
        .iter()
        .zip(b_bytes.iter())
        .map(|(x, y)| x ^ y)
        .collect();
    format!("/chat/{}", hex_encode(&xor))
}

/// Derive the peer's public key from a `/chat/{xor_hex}` path and our own key.
pub fn peer_key_from_chat_path(own_key: &str, path: &str) -> Option<String> {
    let xor_hex = path.strip_prefix("/chat/")?;
    let xor_hex = xor_hex.split('/').next()?;
    let xor_bytes = hex_decode(xor_hex)?;
    let own_bytes = hex_decode(own_key)?;
    if xor_bytes.len() != own_bytes.len() {
        return None;
    }
    let peer: Vec<u8> = xor_bytes
        .iter()
        .zip(own_bytes.iter())
        .map(|(a, b)| a ^ b)
        .collect();
    Some(peer.iter().map(|b| format!("{:02x}", b)).collect())
}
