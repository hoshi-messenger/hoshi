use std::net::SocketAddr;

use axum::{
    Json,
    extract::{ConnectInfo, State},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use hoshi_protocol::control_plane::{
    RegisterRelayRequest, RelayEntry, RelayRegistrationProofPayload,
};

use crate::ServerState;

use super::common::{error_response, serialize_payload, verify_noise_proof};

pub(crate) async fn register_relay_post(
    State(state): State<ServerState>,
    ConnectInfo(peer_addr): ConnectInfo<SocketAddr>,
    Json(payload): Json<RegisterRelayRequest>,
) -> Response {
    if let Err(err) = state.db.validate_relay_api_key(&payload.api_key).await {
        return error_response(StatusCode::UNAUTHORIZED, err.to_string());
    }

    let guid = match uuid::Uuid::parse_str(&payload.guid) {
        Ok(guid) => guid.to_string(),
        Err(_) => return error_response(StatusCode::BAD_REQUEST, "invalid guid"),
    };

    if payload.port == 0 {
        return error_response(StatusCode::BAD_REQUEST, "invalid port");
    }

    let verified = match verify_noise_proof(
        &state,
        &payload.public_key,
        &payload.noise_handshake,
        |canonical_public_key| {
            serialize_payload(&RelayRegistrationProofPayload {
                public_key: canonical_public_key.to_string(),
                guid: guid.clone(),
                api_key: payload.api_key.clone(),
                port: payload.port,
            })
        },
    ) {
        Ok(verified) => verified,
        Err(err) => return err.into_response(),
    };

    let relay = RelayEntry {
        guid: guid.clone(),
        public_key: verified.canonical_public_key,
        ip: peer_addr.ip().to_string(),
        port: payload.port,
    };

    let already_exists = state.relays.insert(guid, relay.clone()).is_some();
    let status = if already_exists {
        StatusCode::OK
    } else {
        StatusCode::CREATED
    };

    (status, Json(relay)).into_response()
}

pub(crate) async fn list_relays_get(State(state): State<ServerState>) -> Response {
    let mut relays: Vec<RelayEntry> = state
        .relays
        .iter()
        .map(|entry| entry.value().clone())
        .collect();
    relays.sort_by(|a, b| a.guid.cmp(&b.guid));

    (StatusCode::OK, Json(relays)).into_response()
}
