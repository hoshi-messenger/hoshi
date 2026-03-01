mod ws;

use axum::{
    Json,
    extract::{State, WebSocketUpgrade},
    http::{HeaderMap, StatusCode, header},
    response::{Html, IntoResponse, Response},
};
use hoshi_protocol::{common::ErrorResponse, relay::HealthzResponse};

use crate::ServerState;

pub async fn index_get(State(_state): State<ServerState>) -> Html<String> {
    Html("<h1>Welcome to the Hoshi relay!</h1>".to_string())
}

pub async fn healthz_get(State(state): State<ServerState>) -> impl IntoResponse {
    Json(HealthzResponse {
        status: "ok".to_string(),
        guid: state.config.guid.clone(),
        control_plane_uri: state.config.control_plane_uri.clone(),
    })
}

pub async fn relay_ws_get(
    State(state): State<ServerState>,
    ws: WebSocketUpgrade,
    headers: HeaderMap,
) -> Response {
    let Some(token) = extract_bearer_token(&headers) else {
        return error_response(StatusCode::UNAUTHORIZED, "missing bearer token");
    };

    if !state.relay_jwt_ready().await {
        return error_response(StatusCode::SERVICE_UNAVAILABLE, "relay jwt key unavailable");
    }

    let identity = match state.verify_relay_jwt(&token).await {
        Ok(identity) => identity,
        Err(err) => return error_response(StatusCode::UNAUTHORIZED, err.to_string()),
    };

    ws.on_upgrade(move |socket| ws::relay_socket_loop(state, socket, identity))
        .into_response()
}

fn extract_bearer_token(headers: &HeaderMap) -> Option<String> {
    let value = headers.get(header::AUTHORIZATION)?;
    let value = value.to_str().ok()?.trim();
    let token = value.strip_prefix("Bearer ")?;

    if token.trim().is_empty() {
        return None;
    }

    Some(token.trim().to_string())
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
