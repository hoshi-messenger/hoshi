#![allow(dead_code, unused_imports)]

use std::{future::Future, path::Path};

use hoshi_relay::{Config, ServerState, create_listeners, run};
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

pub async fn with_backend<F, Fut>(test: F)
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
    let (http_listener, http_addr) = create_listeners(&config).expect("create listeners");
    let config = config.update_bound_addresses(http_addr);
    let state = ServerState::new(config, process_start)
        .await
        .expect("create relay state");

    run(state.clone(), http_listener, test(state)).await;
}
