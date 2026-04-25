use std::convert::Infallible;

use futures::{SinkExt, StreamExt};
use hoshi_clientlib::HoshiEnvelope;
use http_body_util::Full;
use hyper::{
    Method, Request, Response, StatusCode, Version,
    body::{Bytes, Incoming},
    header::{
        ACCEPT, CONNECTION, CONTENT_TYPE, SEC_WEBSOCKET_ACCEPT, SEC_WEBSOCKET_KEY,
        SEC_WEBSOCKET_VERSION, UPGRADE, USER_AGENT,
    },
    upgrade::Upgraded,
};
use hyper_util::rt::TokioIo;
use tokio_tungstenite::{
    WebSocketStream,
    tungstenite::{
        handshake::derive_accept_key,
        protocol::{Message as WsMessage, Role},
    },
};

use crate::{ServerState, api, connection::HoshiConnection, http::TlsConnectInfo};

type Body = Full<Bytes>;
const STATUS_TEMPLATE: &str = include_str!("status.html");
const STATUS_OK: &str = "ok";

pub async fn handle_request(
    state: ServerState,
    conn_info: TlsConnectInfo,
    mut req: Request<Incoming>,
) -> Result<Response<Body>, Infallible> {
    if req.uri().path() != "/" {
        return Ok(empty_response(StatusCode::NOT_FOUND));
    }

    if is_websocket_upgrade_attempt(&req) {
        return Ok(websocket_response(state, conn_info, &mut req));
    }

    match *req.method() {
        Method::GET => Ok(relay_status_response(state, &req)),
        _ => Ok(empty_response(StatusCode::METHOD_NOT_ALLOWED)),
    }
}

fn relay_status_response(state: ServerState, req: &Request<Incoming>) -> Response<Body> {
    let status = relay_status(&state);
    let accepts_html = req
        .headers()
        .get(ACCEPT)
        .and_then(|v| v.to_str().ok())
        .map(|v| v.contains("text/html"))
        .unwrap_or(false);

    if accepts_html {
        response(
            StatusCode::OK,
            "text/html; charset=utf-8",
            render_status_html(&status),
        )
    } else {
        let body = serde_json::to_string(&status).expect("relay status response serializes");
        response(StatusCode::OK, "application/json", body)
    }
}

fn relay_status(state: &ServerState) -> api::RelayStatusResponse {
    let stats = state.stats_snapshot();
    api::RelayStatusResponse {
        status: STATUS_OK.to_string(),
        public_key: state.public_key.clone(),
        connected_clients: stats.connected_clients,
        messages_per_second: stats.messages_per_second,
        bytes_per_second: stats.bytes_per_second,
    }
}

fn render_status_html(status: &api::RelayStatusResponse) -> String {
    STATUS_TEMPLATE
        .replace("<!--PUBLIC_KEY-->", &status.public_key)
        .replace(
            "<!--CONNECTED_CLIENTS-->",
            &status.connected_clients.to_string(),
        )
        .replace(
            "<!--MESSAGES_PER_SECOND-->",
            &status.messages_per_second.to_string(),
        )
        .replace(
            "<!--BYTES_PER_SECOND-->",
            &status.bytes_per_second.to_string(),
        )
}

fn websocket_response(
    state: ServerState,
    conn_info: TlsConnectInfo,
    req: &mut Request<Incoming>,
) -> Response<Body> {
    if !is_valid_websocket_upgrade(req) {
        return empty_response(StatusCode::BAD_REQUEST);
    }

    let user_agent = req
        .headers()
        .get(USER_AGENT)
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default();

    // Just something to get rid of simple bots.
    if !user_agent.contains("Hoshi") {
        return empty_response(StatusCode::UNAUTHORIZED);
    }

    let Some(client_key) = conn_info.client_public_key else {
        eprintln!(
            "WS: no client certificate presented from {}",
            conn_info.remote_addr
        );
        return empty_response(StatusCode::UNAUTHORIZED);
    };

    let Some(ws_accept) = req
        .headers()
        .get(SEC_WEBSOCKET_KEY)
        .map(|key| derive_accept_key(key.as_bytes()))
    else {
        return empty_response(StatusCode::BAD_REQUEST);
    };

    let version = req.version();
    let on_upgrade = hyper::upgrade::on(req);
    tokio::spawn(async move {
        match on_upgrade.await {
            Ok(upgraded) => {
                let socket =
                    WebSocketStream::from_raw_socket(TokioIo::new(upgraded), Role::Server, None)
                        .await;
                handle_ws(state, socket, client_key).await;
            }
            Err(e) => eprintln!("WS upgrade error: {e}"),
        }
    });

    let mut res = Response::new(Body::default());
    *res.status_mut() = StatusCode::SWITCHING_PROTOCOLS;
    *res.version_mut() = version;
    res.headers_mut()
        .insert(CONNECTION, "Upgrade".parse().expect("valid header"));
    res.headers_mut()
        .insert(UPGRADE, "websocket".parse().expect("valid header"));
    res.headers_mut().insert(
        SEC_WEBSOCKET_ACCEPT,
        ws_accept.parse().expect("valid websocket accept header"),
    );
    res
}

fn is_websocket_upgrade_attempt(req: &Request<Incoming>) -> bool {
    req.headers()
        .get(UPGRADE)
        .and_then(|h| h.to_str().ok())
        .map(|h| h.eq_ignore_ascii_case("websocket"))
        .unwrap_or(false)
}

fn is_valid_websocket_upgrade(req: &Request<Incoming>) -> bool {
    let headers = req.headers();
    req.method() == Method::GET
        && req.version() >= Version::HTTP_11
        && headers
            .get(CONNECTION)
            .and_then(|h| h.to_str().ok())
            .map(|h| {
                h.split([',', ' '])
                    .any(|part| part.eq_ignore_ascii_case("upgrade"))
            })
            .unwrap_or(false)
        && headers
            .get(UPGRADE)
            .and_then(|h| h.to_str().ok())
            .map(|h| h.eq_ignore_ascii_case("websocket"))
            .unwrap_or(false)
        && headers
            .get(SEC_WEBSOCKET_VERSION)
            .map(|h| h == "13")
            .unwrap_or(false)
}

fn response(
    status: StatusCode,
    content_type: &'static str,
    body: impl Into<Bytes>,
) -> Response<Body> {
    let mut res = Response::new(Full::new(body.into()));
    *res.status_mut() = status;
    res.headers_mut().insert(
        CONTENT_TYPE,
        content_type.parse().expect("valid content type"),
    );
    res
}

fn empty_response(status: StatusCode) -> Response<Body> {
    let mut res = Response::new(Body::default());
    *res.status_mut() = status;
    res
}

async fn handle_ws(
    state: ServerState,
    socket: WebSocketStream<TokioIo<Upgraded>>,
    client_key: String,
) {
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
                        if let Ok(envelope) = rmp_serde::from_slice::<HoshiEnvelope>(bytes.as_ref()) {
                            state.stats.record_message(bytes.len() as u64);
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
                            if sink.send(WsMessage::binary(bytes)).await.is_err() {
                                break;
                            }
                        }
                    }
                    None => break,
                }
            }
        }
    }

    let should_remove_key = if let Some(mut conns) = state.connections.get_mut(&client_key) {
        conns.retain(|c| c.id != conn_id);
        conns.is_empty()
    } else {
        false
    };
    if should_remove_key {
        state.connections.remove(&client_key);
    }
    println!("WS: [{client_key}] disconnected (conn {conn_id})");
}
