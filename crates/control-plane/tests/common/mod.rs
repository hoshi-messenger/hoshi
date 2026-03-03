#![allow(dead_code, unused_imports)]

use std::future::Future;

use hoshi_control_plane::{Config, ServerState, run};
use hoshi_server_util::create_http_listener;
use tempfile::TempDir;

mod api;
pub use api::*;

/// Runs a control-plane instance for an integration test using a temporary data dir.
///
/// This helper binds to `127.0.0.1:0` so tests can run in parallel.
pub async fn with_control_plane<F, Fut>(test: F)
where
    F: FnOnce(ServerState) -> Fut,
    Fut: Future<Output = ()>,
{
    let process_start = std::time::Instant::now();
    let dir = TempDir::new().expect("Couldn't create TempDir");
    let dir_root = dir
        .path()
        .to_str()
        .expect("Couldn't turn TempDir path to_str()");

    println!("TempDir: {dir_root}");

    // Bind to port 0 for OS-assigned ports
    let config = Config::default()
        .set_dir_root(dir_root)
        .set_db_name(":memory:")
        .set_relay_api_key("test-relay-api-key")
        .set_http_bind_addr("127.0.0.1:0")
        .expect("set_http_bind_addr");

    // Create listeners and get actual ports
    let (http_listener, http_addr) =
        create_http_listener(config.http_bind_address).expect("Failed to create listeners");

    println!("HTTP bound to: {}", http_addr);

    // Update config with actual addresses
    let config = config.update_bound_addresses(http_addr);
    let state = ServerState::new(config, process_start)
        .await
        .expect("State");

    run(state.clone(), http_listener, test(state)).await;
}
