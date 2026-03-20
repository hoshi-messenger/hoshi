use std::net::SocketAddr;

use axum::{
    Router,
    extract::connect_info::Connected,
    routing::any,
    serve::{IncomingStream, Listener},
};
use hoshi_clientlib::identity;
use tokio::net::TcpListener;
use tokio_rustls::TlsAcceptor;
use tokio_rustls::server::TlsStream;

use crate::{ServerState, index_route};

pub struct TlsListener {
    listener: TcpListener,
    acceptor: TlsAcceptor,
}

impl TlsListener {
    pub fn new(listener: TcpListener, acceptor: TlsAcceptor) -> Self {
        Self { listener, acceptor }
    }
}

impl Listener for TlsListener {
    type Io = TlsStream<tokio::net::TcpStream>;
    type Addr = SocketAddr;

    async fn accept(&mut self) -> (Self::Io, Self::Addr) {
        loop {
            match self.listener.accept().await {
                Ok((tcp, addr)) => match self.acceptor.accept(tcp).await {
                    Ok(tls) => return (tls, addr),
                    Err(e) => {
                        eprintln!("TLS handshake error: {e}");
                        continue;
                    }
                },
                Err(e) => {
                    eprintln!("TCP accept error: {e}");
                    continue;
                }
            }
        }
    }

    fn local_addr(&self) -> std::io::Result<Self::Addr> {
        self.listener.local_addr()
    }
}

/// Connection metadata extracted from the TLS handshake, available to handlers
/// via `ConnectInfo<TlsConnectInfo>`.
#[derive(Clone, Debug)]
pub struct TlsConnectInfo {
    pub remote_addr: SocketAddr,
    /// Ed25519 public key hex extracted from the client's TLS certificate.
    pub client_public_key: Option<String>,
}

impl Connected<IncomingStream<'_, TlsListener>> for TlsConnectInfo {
    fn connect_info(stream: IncomingStream<'_, TlsListener>) -> Self {
        let remote_addr = *stream.remote_addr();

        let client_public_key = stream
            .io()
            .get_ref()
            .1
            .peer_certificates()
            .and_then(|certs| certs.first())
            .and_then(|cert| identity::extract_ed25519_public_key_hex(cert.as_ref()));

        TlsConnectInfo {
            remote_addr,
            client_public_key,
        }
    }
}

pub async fn https_server(
    state: ServerState,
    listener: TcpListener,
    tls_acceptor: TlsAcceptor,
) -> anyhow::Result<impl std::future::IntoFuture<Output = Result<(), std::io::Error>>> {
    let bind_addr = listener.local_addr()?;
    let process_start = state.process_start;

    let app = Router::new().route("/", any(index_route)).with_state(state);
    let tls_listener = TlsListener::new(listener, tls_acceptor);

    println!(
        "[{:?}] - Hoshi relay HTTPS ready on {bind_addr}",
        process_start.elapsed()
    );

    Ok(axum::serve(
        tls_listener,
        app.into_make_service_with_connect_info::<TlsConnectInfo>(),
    ))
}
