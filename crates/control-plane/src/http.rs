use std::net::SocketAddr;

use axum::{Router, routing::get};
use tokio::net::TcpListener;

use crate::{ServerState, index_get};

pub async fn http_server(
    state: ServerState,
    listener: TcpListener,
) -> anyhow::Result<impl std::future::IntoFuture<Output = Result<(), std::io::Error>>> {
    let bind_addr = listener.local_addr()?;
    let process_start = state.process_start;

    let app = Router::new()
        .route("/", get(index_get))
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
