use axum::{
    Json,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use serde::Serialize;

use crate::{
    ServerState,
    api::ErrorResponse,
    noise::{
        canonicalize_base64_32, decode_base64, serialize_proof_payload, verify_registration_proof,
    },
};

pub(crate) struct VerifiedNoiseProof {
    pub canonical_public_key: String,
}

pub(crate) struct RouteError {
    status: StatusCode,
    message: String,
}

impl RouteError {
    pub(crate) fn into_response(self) -> Response {
        error_response(self.status, self.message)
    }
}

pub(crate) fn error_response(status: StatusCode, message: impl Into<String>) -> Response {
    (
        status,
        Json(ErrorResponse {
            error: message.into(),
        }),
    )
        .into_response()
}

pub(crate) fn serialize_payload<T: Serialize>(payload: &T) -> Result<Vec<u8>, RouteError> {
    serialize_proof_payload(payload).map_err(|err| RouteError {
        status: StatusCode::INTERNAL_SERVER_ERROR,
        message: err.to_string(),
    })
}

pub(crate) fn verify_noise_proof<F>(
    state: &ServerState,
    public_key_b64: &str,
    noise_handshake_b64: &str,
    build_payload: F,
) -> Result<VerifiedNoiseProof, RouteError>
where
    F: FnOnce(&str) -> Result<Vec<u8>, RouteError>,
{
    let (canonical_public_key, public_key) =
        canonicalize_base64_32(public_key_b64, "public_key").map_err(|err| RouteError {
            status: StatusCode::BAD_REQUEST,
            message: err.to_string(),
        })?;

    let noise_handshake = decode_base64(noise_handshake_b64).map_err(|_| RouteError {
        status: StatusCode::BAD_REQUEST,
        message: "invalid noise_handshake base64".to_string(),
    })?;

    let proof_payload = build_payload(&canonical_public_key)?;

    verify_registration_proof(
        state.noise_static_private_key(),
        &public_key,
        &noise_handshake,
        &proof_payload,
    )
    .map_err(|_| RouteError {
        status: StatusCode::BAD_REQUEST,
        message: "invalid registration proof".to_string(),
    })?;

    Ok(VerifiedNoiseProof {
        canonical_public_key,
    })
}
