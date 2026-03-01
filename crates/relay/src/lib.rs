mod config;
mod http;
mod noise;
mod routes;
mod state;

use std::{future::Future, net::SocketAddr};

pub use config::Config;
pub use state::ServerState;
use tokio::{
    net::{TcpListener, TcpSocket},
    runtime::Builder,
};

pub(crate) use routes::*;

pub async fn run<T: Future>(state: ServerState, http_listener: TcpListener, kill: T) {
    println!(
        "[{:?}] - Hoshi relay started",
        state.process_start.elapsed()
    );

    match state.probe_control_plane().await {
        Ok(status) => {
            println!(
                "[{:?}] - Control-plane probe OK: {} ({status})",
                state.process_start.elapsed(),
                state.config.control_plane_uri
            );
        }
        Err(err) => {
            eprintln!(
                "[{:?}] - Control-plane probe failed: {} ({err})",
                state.process_start.elapsed(),
                state.config.control_plane_uri
            );
        }
    }

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

    let jwt_refresh_task = tokio::spawn(state.clone().run_relay_jwt_key_refresh_loop());
    let relay_registration_task = tokio::spawn(state.clone().run_relay_registration_loop());

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

    jwt_refresh_task.abort();
    relay_registration_task.abort();
}

pub fn create_listener(addr: SocketAddr, reuse_port: bool) -> std::io::Result<TcpListener> {
    let socket = if addr.is_ipv4() {
        TcpSocket::new_v4()?
    } else {
        TcpSocket::new_v6()?
    };

    socket.set_reuseaddr(true)?;

    #[cfg(target_os = "linux")]
    socket.set_reuseport(reuse_port)?;
    #[cfg(not(target_os = "linux"))]
    let _ = reuse_port;

    socket.bind(addr)?;
    socket.listen(1024)
}

pub fn create_listeners(config: &Config) -> std::io::Result<(TcpListener, SocketAddr)> {
    let http_listener = create_listener(config.http_bind_address, config.reuse_port)?;
    let http_addr = http_listener.local_addr()?;
    Ok((http_listener, http_addr))
}

pub fn run_multi_thread(config: Config, process_start: std::time::Instant) {
    let runtime = Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("couldn't start tokio runtime");

    runtime.block_on(async {
        let (http_listener, http_addr) =
            create_listeners(&config).expect("failed to create listeners");
        let config = config.update_bound_addresses(http_addr);

        let state = ServerState::new(config, process_start)
            .await
            .expect("error creating relay state from config");

        let kill = std::future::pending::<()>();
        run(state, http_listener, kill).await;
    });
}
