use std::net::SocketAddr;

use axum::{
    Router,
    routing::{get, post},
};
use tokio::net::TcpListener;

use crate::{
    ServerState, index_get, issue_relay_token_post, list_relays_get, lookup_client_get,
    noise_public_key_get, register_client_post, register_relay_post, relay_jwt_public_key_get,
};

pub async fn http_server(
    state: ServerState,
    listener: TcpListener,
) -> anyhow::Result<impl std::future::IntoFuture<Output = Result<(), std::io::Error>>> {
    let bind_addr = listener.local_addr()?;
    let process_start = state.process_start;

    let app = Router::new()
        .route("/", get(index_get))
        .route("/noise/public-key", get(noise_public_key_get))
        .route("/auth/relay-jwt-public-key", get(relay_jwt_public_key_get))
        .route("/auth/relay-token", post(issue_relay_token_post))
        .route("/clients", post(register_client_post))
        .route("/clients/{guid}", get(lookup_client_get))
        .route("/relays", get(list_relays_get).post(register_relay_post))
        .with_state(state.clone());

    println!(
        "[{:?}] - Hoshi control plane HTTP ready on {bind_addr}",
        process_start.elapsed()
    );

    Ok(axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    ))
}
