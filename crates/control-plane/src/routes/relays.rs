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

use crate::{RelayPresence, ServerState, now};

use super::common::{error_response, serialize_payload, verify_noise_proof};

const RELAY_TTL_SECONDS: i64 = 90;

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
    let relay_presence = RelayPresence {
        entry: relay.clone(),
        last_seen: now(),
    };

    let already_exists = state.relays.insert(guid, relay_presence).is_some();
    let status = if already_exists {
        StatusCode::OK
    } else {
        StatusCode::CREATED
    };

    (status, Json(relay)).into_response()
}

pub(crate) async fn list_relays_get(State(state): State<ServerState>) -> Response {
    let now_ts = now();
    let mut stale_guids = Vec::new();
    let mut relays: Vec<RelayEntry> = Vec::new();

    for relay in state.relays.iter() {
        let relay_age = now_ts.saturating_sub(relay.value().last_seen);
        if relay_age > RELAY_TTL_SECONDS {
            stale_guids.push(relay.key().clone());
            continue;
        }
        relays.push(relay.value().entry.clone());
    }

    for guid in stale_guids {
        state.relays.remove(&guid);
    }

    relays.sort_by(|a, b| a.guid.cmp(&b.guid));

    (StatusCode::OK, Json(relays)).into_response()
}
