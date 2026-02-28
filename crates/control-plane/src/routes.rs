use std::net::SocketAddr;

use axum::{
    Json,
    extract::{ConnectInfo, Path, State},
    http::StatusCode,
    response::{Html, IntoResponse, Response},
};
use base64::{Engine as _, engine::general_purpose::STANDARD};

use crate::api::{
    ErrorResponse, LookupClientResponse, RegisterClientRequest, RegisterRelayRequest, RelayEntry,
};
use crate::{Client, ClientType, ServerState, utils::response_html};

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

fn canonicalize_base64(value: &str) -> anyhow::Result<String> {
    let decoded = STANDARD.decode(value)?;
    Ok(STANDARD.encode(decoded))
}

pub async fn register_client_post(
    State(state): State<ServerState>,
    Json(payload): Json<RegisterClientRequest>,
) -> Response {
    let canonical_public_key = match canonicalize_base64(&payload.public_key) {
        Ok(value) => value,
        Err(_) => return error_response(StatusCode::BAD_REQUEST, "invalid public_key base64"),
    };

    if matches!(payload.client_type, ClientType::Relay) {
        return error_response(StatusCode::BAD_REQUEST, "relay is not allowed in /clients");
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

    let canonical_public_key = match canonicalize_base64(&payload.public_key) {
        Ok(value) => value,
        Err(_) => return error_response(StatusCode::BAD_REQUEST, "invalid public_key base64"),
    };

    if payload.port == 0 {
        return error_response(StatusCode::BAD_REQUEST, "invalid port");
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
