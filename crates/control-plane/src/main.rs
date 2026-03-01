use hoshi_control_plane::{Config, run_multi_thread};

fn main() {
    let start = std::time::Instant::now();
    let config = Config::new();

    run_multi_thread(config, start);
}
