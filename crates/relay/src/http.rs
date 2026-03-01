use std::net::SocketAddr;

use axum::{Router, routing::get};
use tokio::net::TcpListener;

use crate::{ServerState, healthz_get, index_get, relay_ws_get};

pub fn router(state: ServerState) -> Router {
    Router::new()
        .route("/", get(index_get))
        .route("/healthz", get(healthz_get))
        .route("/relay", get(relay_ws_get))
        .with_state(state)
}

pub async fn http_server(
    state: ServerState,
    listener: TcpListener,
) -> anyhow::Result<impl std::future::IntoFuture<Output = Result<(), std::io::Error>>> {
    let bind_addr = listener.local_addr()?;
    let process_start = state.process_start;
    let app = router(state);

    println!(
        "[{:?}] - Hoshi relay HTTP ready on {bind_addr}",
        process_start.elapsed()
    );

    Ok(axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    ))
}
