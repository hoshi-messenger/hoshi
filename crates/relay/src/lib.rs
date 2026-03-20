pub mod api;
mod config;
pub(crate) mod connection;
mod http;
mod routes;
mod state;
pub mod tls;

use std::future::Future;
use std::net::SocketAddr;

pub use config::Config;
pub use state::ServerState;
use tokio::{
    net::{TcpListener, TcpSocket},
    runtime::Builder,
};
use tokio_rustls::TlsAcceptor;

fn create_listener(addr: SocketAddr) -> std::io::Result<TcpListener> {
    let socket = if addr.is_ipv4() {
        TcpSocket::new_v4()?
    } else {
        TcpSocket::new_v6()?
    };
    socket.set_reuseaddr(true)?;
    socket.bind(addr)?;
    socket.listen(1024)
}

pub fn create_http_listener(addr: SocketAddr) -> std::io::Result<(TcpListener, SocketAddr)> {
    let http_listener = create_listener(addr)?;
    let http_addr = http_listener.local_addr()?;
    Ok((http_listener, http_addr))
}

fn sd_notify_ready() {
    #[cfg(target_os = "linux")]
    {
        use sd_notify::{NotifyState, notify};
        use std::time::Duration;
        use tokio::time::interval;

        let _ = notify(false, &[NotifyState::Ready]);

        let watchdog_usec = match std::env::var("WATCHDOG_USEC") {
            Ok(v) => v.parse::<u64>().ok(),
            Err(_) => None,
        };

        let watchdog_usec = match watchdog_usec {
            Some(v) if v > 0 => v,
            _ => return,
        };

        let interval_duration = Duration::from_micros(watchdog_usec / 2);

        tokio::spawn(async move {
            let mut ticker = interval(interval_duration);
            loop {
                ticker.tick().await;
                let _ = notify(false, &[NotifyState::Watchdog]);
            }
        });
    }
}

pub(crate) use routes::*;

pub async fn run<T: Future>(
    state: ServerState,
    http_listener: TcpListener,
    tls_acceptor: TlsAcceptor,
    kill: T,
) {
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

    let https_server = http::https_server(state.clone(), http_listener, tls_acceptor)
        .await
        .expect("couldn't start relay https server");

    println!("[{:?}] - Hoshi relay ready", state.process_start.elapsed());

    sd_notify_ready();

    tokio::select! {
        http_res = https_server => {
            eprintln!("HTTPS server stopped: {:?}", http_res);
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
        let identity = tls::load_or_generate_identity(&config)
            .expect("failed to load or generate relay identity");
        let tls_acceptor =
            tls::create_tls_acceptor(&identity).expect("failed to create TLS acceptor");

        println!("Relay public key: {}", identity.public_key_hex());

        let (http_listener, http_addr) =
            create_http_listener(config.http_bind_address).expect("failed to create listeners");
        let config = config.update_bound_addresses(http_addr);

        let state = ServerState::new(config, process_start, identity.public_key_hex())
            .await
            .expect("error creating relay state from config");

        let kill = std::future::pending::<()>();
        run(state, http_listener, tls_acceptor, kill).await;
    });
}
