pub mod api;
mod config;
pub(crate) mod connection;
mod http;
mod routes;
mod state;

use std::future::Future;

pub use config::Config;
use hoshi_server_util::create_http_listener;
pub use state::ServerState;
use tokio::{net::TcpListener, runtime::Builder};

pub(crate) use routes::*;

pub async fn run<T: Future>(state: ServerState, http_listener: TcpListener, kill: T) {
    println!(
        "[{:?}] - Hoshi relay started",
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
        .expect("couldn't start relay http server");

    println!("[{:?}] - Hoshi relay ready", state.process_start.elapsed());

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

pub fn run_multi_thread(config: Config, process_start: std::time::Instant) {
    let runtime = Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("couldn't start tokio runtime");

    runtime.block_on(async {
        let (http_listener, http_addr) =
            create_http_listener(config.http_bind_address).expect("failed to create listeners");
        let config = config.update_bound_addresses(http_addr);

        let state = ServerState::new(config, process_start)
            .await
            .expect("error creating relay state from config");

        let kill = std::future::pending::<()>();
        run(state, http_listener, kill).await;
    });
}
