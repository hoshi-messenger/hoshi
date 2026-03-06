use axum::{
    Json,
    extract::{
        State, WebSocketUpgrade,
        ws::{Message as WsMessage, WebSocket, rejection::WebSocketUpgradeRejection},
    },
    http::{HeaderMap, Method, StatusCode},
    response::{Html, IntoResponse},
};
use futures::{SinkExt, StreamExt};
use hoshi_clientlib::HoshiEnvelope;

use crate::{ServerState, api, connection::HoshiConnection};

#[axum::debug_handler]
pub async fn index_route(
    State(state): State<ServerState>,
    method: Method,
    headers: HeaderMap,
    ws: Result<WebSocketUpgrade, WebSocketUpgradeRejection>,
) -> impl IntoResponse {
    match ws {
        Ok(upgrade) => {
            let client_key = headers
                .get("authorization")
                .and_then(|v| v.to_str().ok())
                .and_then(|v| v.strip_prefix("Bearer "))
                .map(|s| s.to_string());

            let Some(client_key) = client_key else {
                return StatusCode::UNAUTHORIZED.into_response();
            };

            upgrade.on_upgrade(async move |socket| {
                handle_ws(state, socket, client_key).await;
            })
        }
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

async fn handle_ws(state: ServerState, socket: WebSocket, client_key: String) {
    let (mut sink, mut stream) = socket.split();
    let (tx, mut rx) = tokio::sync::mpsc::channel::<HoshiEnvelope>(64);

    let conn_id = uuid::Uuid::new_v7(uuid::Timestamp::now(uuid::NoContext));
    state
        .connections
        .entry(client_key.clone())
        .or_default()
        .push(HoshiConnection { id: conn_id, tx });

    println!("WS: [{client_key}] connected (conn {conn_id})");

    loop {
        tokio::select! {
            // Incoming from this client's WebSocket
            msg = stream.next() => {
                match msg {
                    Some(Ok(WsMessage::Binary(bytes))) => {
                        if let Ok(envelope) = rmp_serde::from_slice::<HoshiEnvelope>(&bytes) {
                            println!("WS: [{client_key}] -> [{}] ({} bytes)", envelope.recipient, bytes.len());
                            if let Some(conns) = state.connections.get(&envelope.recipient) {
                                println!("WS: routing to {} connection(s) for [{}]", conns.len(), envelope.recipient);
                                for conn in conns.iter() {
                                    let _ = conn.tx.try_send(envelope.clone());
                                }
                            } else {
                                println!("WS: no connections found for [{}]", envelope.recipient);
                            }
                        } else {
                            println!("WS: [{client_key}] failed to deserialize envelope ({} bytes)", bytes.len());
                        }
                    }
                    Some(Ok(_)) => {} // ignore text/ping/pong frames
                    _ => break,       // error or close
                }
            }
            // Outgoing: envelope routed to this client by the relay
            env = rx.recv() => {
                match env {
                    Some(envelope) => {
                        println!("WS: forwarding envelope to [{client_key}] ({} payload bytes)", envelope.payload.len());
                        if let Ok(bytes) = rmp_serde::to_vec(&envelope) {
                            if sink.send(WsMessage::Binary(bytes.into())).await.is_err() {
                                break;
                            }
                        }
                    }
                    None => break,
                }
            }
        }
    }

    // Cleanup: remove this connection from the map
    if let Some(mut conns) = state.connections.get_mut(&client_key) {
        conns.retain(|c| c.id != conn_id);
    }
    println!("WS: [{client_key}] disconnected (conn {conn_id})");
}
