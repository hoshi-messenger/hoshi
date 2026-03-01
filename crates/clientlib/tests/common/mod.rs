#![allow(dead_code, unused_imports)]

use std::{future::Future, path::Path};

use hoshi_control_plane::{
    Config as ControlPlaneConfig, ServerState as ControlPlaneState,
    create_listeners as create_control_plane_listeners, run as run_control_plane,
};
use hoshi_relay::{
    Config as RelayConfig, ServerState as RelayState, create_listeners as create_relay_listeners,
    run as run_relay,
};
use tempfile::TempDir;

fn write_relay_config(
    path: &Path,
    api_key: &str,
    bind_addr: &str,
    control_plane_uri: &str,
) -> std::io::Result<()> {
    let config = format!(
        r#"
http_bind_address = "{bind_addr}"
reuse_port = false
control_plane_uri = "{control_plane_uri}"
api_key = "{api_key}"
"#
    );
    std::fs::write(path, config)
}

pub async fn with_control_plane<F, Fut>(test: F)
where
    F: FnOnce(ControlPlaneState) -> Fut,
    Fut: Future<Output = ()>,
{
    let process_start = std::time::Instant::now();
    let dir = TempDir::new().expect("couldn't create tempdir");
    let dir_root = dir.path().to_str().expect("tempdir path");

    let config = ControlPlaneConfig::default()
        .set_dir_root(dir_root)
        .set_db_name(":memory:")
        .set_relay_api_key("test-relay-api-key")
        .set_http_bind_addr("127.0.0.1:0")
        .expect("set_http_bind_addr");

    let (http_listener, http_addr) =
        create_control_plane_listeners(&config).expect("create control-plane listeners");
    let config = config.update_bound_addresses(http_addr);
    let state = ControlPlaneState::new(config, process_start)
        .await
        .expect("create control-plane state");

    run_control_plane(state.clone(), http_listener, test(state)).await;
}

pub async fn with_relay<F, Fut>(control_plane_uri: &str, api_key: &str, test: F)
where
    F: FnOnce(RelayState) -> Fut,
    Fut: Future<Output = ()>,
{
    let process_start = std::time::Instant::now();
    let dir = TempDir::new().expect("couldn't create tempdir");
    let config_path = dir.path().join("relay.toml");

    write_relay_config(&config_path, api_key, "127.0.0.1:0", control_plane_uri)
        .expect("write relay config");

    let config = RelayConfig::load_from_path(&config_path).expect("load relay config");
    let (http_listener, http_addr) =
        create_relay_listeners(&config).expect("create relay listeners");
    let config = config.update_bound_addresses(http_addr);
    let state = RelayState::new(config, process_start)
        .await
        .expect("create relay state");

    run_relay(state.clone(), http_listener, test(state)).await;
}

pub async fn with_control_plane_and_relay<F, Fut>(test: F)
where
    F: FnOnce(ControlPlaneState, RelayState) -> Fut,
    Fut: Future<Output = ()>,
{
    with_control_plane(|control_plane_state| async move {
        let control_plane_uri = control_plane_state.config.uri();
        let relay_api_key = control_plane_state
            .config
            .relay_api_key
            .clone()
            .expect("control-plane relay_api_key should be set");

        with_relay(
            &control_plane_uri,
            &relay_api_key,
            |relay_state| async move {
                test(control_plane_state, relay_state).await;
            },
        )
        .await;
    })
    .await;
}
