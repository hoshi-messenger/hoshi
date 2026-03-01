mod client;
mod config;
mod database;
mod http;
mod routes;
mod state;
mod utils;

use std::net::SocketAddr;

pub use client::{Client, ClientType};
pub use config::Config;
pub use database::Database;
use hoshi_server_util::{
    create_http_listener, create_listener as create_tcp_listener,
    systemd_notify_ready_with_watchdog,
};
pub(crate) use routes::*;

pub use state::{RelayPresence, ServerState};
use tokio::{net::TcpListener, runtime::Builder};
pub(crate) use utils::now;

pub async fn run<T: Future>(state: ServerState, http_listener: TcpListener, kill: T) {
    println!(
        "[{:?}] - Hoshi control plane started",
        state.process_start.elapsed()
    );

    #[cfg(unix)]
    let terminate = async {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("failed to install signal handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    let http_server = http::http_server(state.clone(), http_listener)
        .await
        .expect("Couldn't start http_server");

    println!(
        "[{:?}] - Hoshi control plane ready",
        state.process_start.elapsed()
    );

    systemd_notify_ready_with_watchdog();

    tokio::select! {
        http_res = http_server => {
            eprintln!("HTTP server stopped: {:?}", http_res);
        }
        signal_res = tokio::signal::ctrl_c() => {
            eprintln!("Received Signal: {:?}", signal_res);
        }
        term_res = terminate => {
            eprintln!("Received Terminate Signal: {:?}", term_res);
        }
        _ = kill => {
            eprintln!("Received Kill!");
        }
    }
}

/// Create and bind a TCP listener with appropriate socket options
pub fn create_listener(addr: SocketAddr, reuse_port: bool) -> std::io::Result<TcpListener> {
    create_tcp_listener(addr, reuse_port)
}

/// Create HTTP listener, returning listener and their bound address
pub fn create_listeners(config: &Config) -> std::io::Result<(TcpListener, SocketAddr)> {
    create_http_listener(config.http_bind_address, config.reuse_port)
}

pub fn run_multi_thread(config: Config, process_start: std::time::Instant) {
    let runtime = Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("Couldn't start tokio runtime");

    runtime.block_on(async {
        // Create listeners inside the runtime
        let (http_listener, http_addr) =
            create_listeners(&config).expect("Failed to create listeners");

        // Update config with actual addresses
        let config = config.update_bound_addresses(http_addr);

        let state = ServerState::new(config, process_start)
            .await
            .expect("Error creating State from Config");

        let kill = std::future::pending::<()>();
        run(state, http_listener, kill).await
    });
}
