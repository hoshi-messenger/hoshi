#![allow(dead_code, unused_imports)]

use std::future::Future;

use hoshi_control_plane::{Config, ServerState, create_listeners, run};
use tempfile::TempDir;

/// Helper function to run integration tests with a temporary backend
///
/// This function:
/// - Verifies git is available (with helpful error if not)
/// - Creates a temporary directory for test data
/// - Binds to port 0 for OS-assigned ports (enables parallel tests)
/// - Passes the State to the test callback
/// - Cleans up after the test completes
pub async fn with_backend<F, Fut>(test: F)
where
    F: FnOnce(ServerState) -> Fut,
    Fut: Future<Output = ()>,
{
    let process_start = std::time::Instant::now();
    let dir = TempDir::new().expect("Couldn't create TempDir");
    let path = dir.path();
    let dir_root = path.to_str().expect("Couldn't turn TempDir path to_str()");

    println!("TempDir: {dir_root}");

    // Bind to port 0 for OS-assigned ports
    let config = Config::default()
        .set_dir_root(dir_root)
        .set_db_name(":memory:")
        .set_http_bind_addr("127.0.0.1:0")
        .expect("set_http_bind_addr");

    // Create listeners and get actual ports
    let (http_listener, http_addr) =
        create_listeners(&config).expect("Failed to create listeners");

    println!("HTTP bound to: {}", http_addr);

    // Update config with actual addresses
    let config = config.update_bound_addresses(http_addr);
    let state = ServerState::new(config, process_start).expect("State");

    // Pass state to test - it can access:
    // - state.config.base_url for HTTP requests
    // - state.config.ssh_public_host for SSH URLs
    // - Any other config as needed
    run(state.clone(), http_listener, test(state)).await;

    std::fs::remove_dir_all(path).expect("Couldn't clean up TempDir");
}
