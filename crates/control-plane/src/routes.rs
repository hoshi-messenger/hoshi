use axum::{
    Json,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    response::{Html, IntoResponse, Response},
};
use base64::{Engine as _, engine::general_purpose::STANDARD};

use crate::api::{
    ErrorResponse, LookupClientResponse, RegisterClientRequest, RegisterRelayRequest,
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
    State(_state): State<ServerState>,
    headers: HeaderMap,
    Json(payload): Json<RegisterRelayRequest>,
) -> Response {
    let Some(api_key) = headers
        .get("x-api-key")
        .and_then(|value| value.to_str().ok())
    else {
        return error_response(StatusCode::UNAUTHORIZED, "missing x-api-key");
    };

    if api_key.trim().is_empty() {
        return error_response(StatusCode::UNAUTHORIZED, "missing x-api-key");
    }

    if canonicalize_base64(&payload.public_key).is_err() {
        return error_response(StatusCode::BAD_REQUEST, "invalid public_key base64");
    }

    error_response(
        StatusCode::NOT_IMPLEMENTED,
        "relay registration not implemented yet",
    )
}
