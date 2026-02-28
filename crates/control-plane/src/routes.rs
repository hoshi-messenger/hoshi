use std::net::SocketAddr;

use axum::{
    Json,
    extract::{ConnectInfo, Path, State},
    http::StatusCode,
    response::{Html, IntoResponse, Response},
};
use serde::Serialize;

use crate::api::{
    ErrorResponse, LookupClientResponse, NoisePublicKeyResponse, RegisterClientRequest,
    RegisterRelayRequest, RelayEntry,
};
use crate::noise::{
    REGISTRATION_NOISE_PATTERN, canonicalize_base64_32, decode_base64, serialize_proof_payload,
    verify_registration_proof,
};
use crate::{Client, ClientType, ServerState, utils::response_html};

#[derive(Serialize)]
struct ClientRegistrationProofPayload<'a> {
    public_key: &'a str,
    owner_id: Option<&'a str>,
    client_type: &'a ClientType,
}

#[derive(Serialize)]
struct RelayRegistrationProofPayload<'a> {
    public_key: &'a str,
    guid: &'a str,
    api_key: &'a str,
    port: u16,
}

pub async fn index_get(State(_state): State<ServerState>) -> Html<String> {
    let html = "<h1>Welcome to the Hoshi control plane!</h1>";
    response_html(html, "Hoshi Control Plane")
}

fn error_response(status: StatusCode, message: impl Into<String>) -> Response {
    (
        status,
        Json(ErrorResponse {
            error: message.into(),
        }),
    )
        .into_response()
}

pub async fn noise_public_key_get(State(state): State<ServerState>) -> Response {
    (
        StatusCode::OK,
        Json(NoisePublicKeyResponse {
            pattern: REGISTRATION_NOISE_PATTERN.to_string(),
            public_key: state.noise_public_key().to_string(),
        }),
    )
        .into_response()
}

pub async fn register_client_post(
    State(state): State<ServerState>,
    Json(payload): Json<RegisterClientRequest>,
) -> Response {
    let (canonical_public_key, public_key) =
        match canonicalize_base64_32(&payload.public_key, "public_key") {
            Ok(value) => value,
            Err(err) => return error_response(StatusCode::BAD_REQUEST, err.to_string()),
        };

    if matches!(payload.client_type, ClientType::Relay) {
        return error_response(StatusCode::BAD_REQUEST, "relay is not allowed in /clients");
    }

    let noise_handshake = match decode_base64(&payload.noise_handshake) {
        Ok(value) => value,
        Err(_) => {
            return error_response(StatusCode::BAD_REQUEST, "invalid noise_handshake base64");
        }
    };

    let proof_payload = ClientRegistrationProofPayload {
        public_key: &canonical_public_key,
        owner_id: payload.owner_id.as_deref(),
        client_type: &payload.client_type,
    };
    let proof_payload = match serialize_proof_payload(&proof_payload) {
        Ok(value) => value,
        Err(err) => return error_response(StatusCode::INTERNAL_SERVER_ERROR, err.to_string()),
    };
    if verify_registration_proof(
        state.noise_static_private_key(),
        &public_key,
        &noise_handshake,
        &proof_payload,
    )
    .is_err()
    {
        return error_response(StatusCode::BAD_REQUEST, "invalid registration proof");
    }

    match state.db.get_client_by_public_key(&canonical_public_key) {
        Ok(Some(_)) => return error_response(StatusCode::CONFLICT, "client already exists"),
        Ok(None) => {}
        Err(err) => return error_response(StatusCode::INTERNAL_SERVER_ERROR, err.to_string()),
    }

    let client = Client::create_client(
        payload.owner_id.as_deref(),
        payload.client_type,
        &canonical_public_key,
    );

    if let Err(err) = state.db.insert_client(&client) {
        return error_response(StatusCode::INTERNAL_SERVER_ERROR, err.to_string());
    }

    (StatusCode::CREATED, Json(client)).into_response()
}

pub async fn lookup_client_get(
    Path(guid): Path<String>,
    State(state): State<ServerState>,
) -> Response {
    let (client, children) = match state.db.get_client_with_children(&guid) {
        Ok(Some(result)) => result,
        Ok(None) => return error_response(StatusCode::NOT_FOUND, "client not found"),
        Err(err) => return error_response(StatusCode::INTERNAL_SERVER_ERROR, err.to_string()),
    };

    let body = LookupClientResponse { client, children };
    (StatusCode::OK, Json(body)).into_response()
}

pub async fn register_relay_post(
    State(state): State<ServerState>,
    ConnectInfo(peer_addr): ConnectInfo<SocketAddr>,
    Json(payload): Json<RegisterRelayRequest>,
) -> Response {
    if let Err(err) = state.db.validate_relay_api_key(&payload.api_key) {
        return error_response(StatusCode::UNAUTHORIZED, err.to_string());
    }

    let guid = match uuid::Uuid::parse_str(&payload.guid) {
        Ok(guid) => guid.to_string(),
        Err(_) => return error_response(StatusCode::BAD_REQUEST, "invalid guid"),
    };

    let (canonical_public_key, public_key) =
        match canonicalize_base64_32(&payload.public_key, "public_key") {
            Ok(value) => value,
            Err(err) => return error_response(StatusCode::BAD_REQUEST, err.to_string()),
        };

    if payload.port == 0 {
        return error_response(StatusCode::BAD_REQUEST, "invalid port");
    }

    let noise_handshake = match decode_base64(&payload.noise_handshake) {
        Ok(value) => value,
        Err(_) => {
            return error_response(StatusCode::BAD_REQUEST, "invalid noise_handshake base64");
        }
    };

    let proof_payload = RelayRegistrationProofPayload {
        public_key: &canonical_public_key,
        guid: &guid,
        api_key: &payload.api_key,
        port: payload.port,
    };
    let proof_payload = match serialize_proof_payload(&proof_payload) {
        Ok(value) => value,
        Err(err) => return error_response(StatusCode::INTERNAL_SERVER_ERROR, err.to_string()),
    };
    if verify_registration_proof(
        state.noise_static_private_key(),
        &public_key,
        &noise_handshake,
        &proof_payload,
    )
    .is_err()
    {
        return error_response(StatusCode::BAD_REQUEST, "invalid registration proof");
    }

    let relay = RelayEntry {
        guid: guid.clone(),
        public_key: canonical_public_key,
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

pub async fn list_relays_get(State(state): State<ServerState>) -> Response {
    let mut relays: Vec<RelayEntry> = state
        .relays
        .iter()
        .map(|entry| entry.value().clone())
        .collect();
    relays.sort_by(|a, b| a.guid.cmp(&b.guid));

    (StatusCode::OK, Json(relays)).into_response()
}
