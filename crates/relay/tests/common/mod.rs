#![allow(dead_code, unused_imports)]

use std::{future::Future, path::Path};

use hoshi_clientlib::identity::HoshiIdentity;
use hoshi_relay::{Config, ServerState, create_http_listener, run, tls};
use tempfile::TempDir;

mod api;
pub use api::*;

pub fn write_test_config(
    path: &Path,
    api_key: &str,
    bind_addr: &str,
    control_plane_uri: &str,
) -> anyhow::Result<()> {
    let config = format!(
        r#"
http_bind_address = "{bind_addr}"
reuse_port = false
control_plane_uri = "{control_plane_uri}"
api_key = "{api_key}"
"#
    );
    std::fs::write(path, config)?;
    Ok(())
}

pub async fn with_relay<F, Fut>(test: F)
where
    F: FnOnce(ServerState) -> Fut,
    Fut: Future<Output = ()>,
{
    let process_start = std::time::Instant::now();
    let dir = TempDir::new().expect("couldn't create tempdir");
    let config_path = dir.path().join("relay.toml");

    write_test_config(
        &config_path,
        "test-relay-api-key",
        "127.0.0.1:0",
        "http://127.0.0.1:1",
    )
    .expect("write test relay config");

    let config = Config::load_from_path(&config_path).expect("load relay config");
    let (http_listener, http_addr) =
        create_http_listener(config.http_bind_address).expect("create listeners");
    let config = config.update_bound_addresses(http_addr);

    let identity = HoshiIdentity::generate();
    let tls_acceptor = tls::create_tls_acceptor(&identity).expect("create TLS acceptor");

    let state = ServerState::new(config, process_start, identity.public_key_hex())
        .await
        .expect("create relay state");

    run(state.clone(), http_listener, tls_acceptor, test(state)).await;
}
