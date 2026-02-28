use std::net::SocketAddr;

use axum::{Router, response::Html, routing::get};
use tokio::net::TcpListener;

use crate::State;

pub async fn http_server(
    state: State,
    listener: TcpListener,
) -> anyhow::Result<impl std::future::IntoFuture<Output = Result<(), std::io::Error>>> {
    let bind_addr = listener.local_addr()?;
    let process_start = state.process_start;

    // build our application with a single route
    let app = Router::new()
        .route("/", get(|| async move {
            Html("<!DOCTYPE html>\n<html><body><h1>Hoshi control plane!</h1></body></html>")
        }))
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
