use axum::{
    Json,
    extract::{
        State, WebSocketUpgrade,
        ws::{WebSocket, rejection::WebSocketUpgradeRejection},
    },
    http::{HeaderMap, Method, StatusCode},
    response::{Html, IntoResponse},
};

use crate::{ServerState, api};

#[axum::debug_handler]
pub async fn index_route(
    State(state): State<ServerState>,
    method: Method,
    headers: HeaderMap,
    ws: Result<WebSocketUpgrade, WebSocketUpgradeRejection>,
) -> impl IntoResponse {
    match ws {
        Ok(upgrade) => upgrade.on_upgrade(async move |mut socket| {
            handle_ws(state, socket).await;
        }),
        Err(_) => match method {
            Method::GET => {
                let accepts_html = headers
                    .get("accept")
                    .and_then(|v| v.to_str().ok())
                    .map(|v| v.contains("text/html"))
                    .unwrap_or(false);

                if accepts_html {
                    relay_status_html(state).await.into_response()
                } else {
                    relay_status_json(state).await.into_response()
                }
            }
            _ => StatusCode::METHOD_NOT_ALLOWED.into_response(),
        },
    }
}

async fn relay_status_html(_state: ServerState) -> impl IntoResponse {
    Html("<h1>Welcome to the Hoshi relay!</h1>".to_string())
}

async fn relay_status_json(_state: ServerState) -> impl IntoResponse {
    Json(api::RelayStatusResponse {
        status: "ok".to_string(),
        public_key: "TEST".to_string(),
    })
}

async fn handle_ws(_state: ServerState, mut _socket: WebSocket) {}
