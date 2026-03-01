use axum::{
    Json,
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use hoshi_protocol::control_plane::{
    ClientType, IssueRelayTokenRequest, IssueRelayTokenResponse, RelayJwtPublicKeyResponse,
};
use jsonwebtoken::{Algorithm, Header, encode};
use serde::{Deserialize, Serialize};

use crate::{ServerState, now};

use super::common::{error_response, serialize_payload, verify_noise_proof};

#[derive(Serialize)]
struct RelayTokenProofPayload<'a> {
    public_key: &'a str,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct RelayTokenClaims {
    sub: String,
    exp: i64,
    iat: i64,
    client_type: ClientType,
}

pub(crate) async fn relay_jwt_public_key_get(State(state): State<ServerState>) -> Response {
    (
        StatusCode::OK,
        Json(RelayJwtPublicKeyResponse {
            alg: "EdDSA".to_string(),
            x: state.relay_jwt_public_key_x().to_string(),
        }),
    )
        .into_response()
}

pub(crate) async fn issue_relay_token_post(
    State(state): State<ServerState>,
    Json(payload): Json<IssueRelayTokenRequest>,
) -> Response {
    let verified = match verify_noise_proof(
        &state,
        &payload.public_key,
        &payload.noise_handshake,
        |canonical_public_key| {
            serialize_payload(&RelayTokenProofPayload {
                public_key: canonical_public_key,
            })
        },
    ) {
        Ok(verified) => verified,
        Err(err) => return err.into_response(),
    };

    let client = match state
        .db
        .get_client_by_public_key(&verified.canonical_public_key)
        .await
    {
        Ok(Some(client)) => client,
        Ok(None) => return error_response(StatusCode::UNAUTHORIZED, "unknown client"),
        Err(err) => return error_response(StatusCode::INTERNAL_SERVER_ERROR, err.to_string()),
    };

    let guid = client.id.clone();
    if uuid::Uuid::parse_str(&guid).is_err() {
        return error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "stored client guid is invalid",
        );
    }

    let issued_at = now();
    let expires_at = issued_at + 86_400;
    let claims = RelayTokenClaims {
        sub: guid.clone(),
        exp: expires_at,
        iat: issued_at,
        client_type: client.client_type.clone().into(),
    };

    let token = match encode(
        &Header::new(Algorithm::EdDSA),
        &claims,
        state.relay_jwt_encoding_key(),
    ) {
        Ok(token) => token,
        Err(err) => return error_response(StatusCode::INTERNAL_SERVER_ERROR, err.to_string()),
    };

    (
        StatusCode::OK,
        Json(IssueRelayTokenResponse {
            token,
            expires_at,
            guid,
            client_type: client.client_type.into(),
        }),
    )
        .into_response()
}
