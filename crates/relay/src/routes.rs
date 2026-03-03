use axum::{
    Json,
    extract::{State, WebSocketUpgrade},
    http::{HeaderMap, StatusCode, header},
    response::{Html, IntoResponse, Response},
};
use hoshi_protocol::relay::HealthzResponse;

use crate::ServerState;

pub async fn index_get(State(_state): State<ServerState>) -> Html<String> {
    Html("<h1>Welcome to the Hoshi relay!</h1>".to_string())
}

pub async fn healthz_get(State(state): State<ServerState>) -> impl IntoResponse {
    Json(HealthzResponse {
        status: "ok".to_string(),
        public_key: "TEST".to_string(),
        control_plane_uri: state.config.control_plane_uri.clone(),
    })
}

pub async fn relay_ws_get(
    State(_state): State<ServerState>,
    ws: WebSocketUpgrade,
    headers: HeaderMap,
) -> Response {
    let Some(_token) = extract_bearer_token(&headers) else {
        return error_response(StatusCode::UNAUTHORIZED, "missing bearer token");
    };

    ws.on_upgrade(async move |mut socket| {
        loop {
            let msg = socket.recv().await;
            if let Some(Ok(msg)) = msg {
                if socket.send(msg).await.is_err() {
                    break;
                }
            }
        }
    }).into_response()
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
        message.into(),
    )
        .into_response()
}
