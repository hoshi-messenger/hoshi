use std::{future::Future, net::SocketAddr};

use hoshi_clientlib::identity;
use hyper::{server::conn::http1, service::service_fn};
use hyper_util::rt::TokioIo;
use tokio::net::TcpListener;
use tokio_rustls::TlsAcceptor;
use tokio_rustls::server::TlsStream;

use crate::{ServerState, handle_request};

#[derive(Clone, Debug)]
pub struct TlsConnectInfo {
    pub remote_addr: SocketAddr,
    pub client_public_key: Option<String>,
}

fn tls_connect_info(
    remote_addr: SocketAddr,
    stream: &TlsStream<tokio::net::TcpStream>,
) -> TlsConnectInfo {
    let client_public_key = stream
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

pub async fn https_server(
    state: ServerState,
    listener: TcpListener,
    tls_acceptor: TlsAcceptor,
) -> anyhow::Result<impl Future<Output = Result<(), std::io::Error>>> {
    let bind_addr = listener.local_addr()?;
    let process_start = state.process_start;

    println!(
        "[{:?}] - Hoshi relay HTTPS ready on {bind_addr}",
        process_start.elapsed()
    );

    Ok(async move {
        loop {
            let (tcp, remote_addr) = match listener.accept().await {
                Ok(conn) => conn,
                Err(e) => {
                    eprintln!("TCP accept error: {e}");
                    continue;
                }
            };

            let tls_acceptor = tls_acceptor.clone();
            let state = state.clone();

            tokio::spawn(async move {
                let tls = match tls_acceptor.accept(tcp).await {
                    Ok(tls) => tls,
                    Err(e) => {
                        eprintln!("TLS handshake error: {e}");
                        return;
                    }
                };

                let conn_info = tls_connect_info(remote_addr, &tls);
                let service =
                    service_fn(move |req| handle_request(state.clone(), conn_info.clone(), req));

                let conn = http1::Builder::new()
                    .serve_connection(TokioIo::new(tls), service)
                    .with_upgrades();

                if let Err(e) = conn.await {
                    eprintln!("HTTP connection error from {remote_addr}: {e}");
                }
            });
        }
    })
}
