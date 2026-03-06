use std::net::SocketAddr;

use axum::{Router, routing::any};
use tokio::net::TcpListener;

use crate::{ServerState, index_route};

pub async fn http_server(
    state: ServerState,
    listener: TcpListener,
) -> anyhow::Result<impl std::future::IntoFuture<Output = Result<(), std::io::Error>>> {
    let bind_addr = listener.local_addr()?;
    let process_start = state.process_start;

    let app = Router::new().route("/", any(index_route)).with_state(state);

    println!(
        "[{:?}] - Hoshi relay HTTP ready on {bind_addr}",
        process_start.elapsed()
    );

    Ok(axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    ))
}
