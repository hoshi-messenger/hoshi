use hoshi_relay::{Config, run_multi_thread};

fn main() {
    let start = std::time::Instant::now();
    let config = Config::new().expect("error creating relay config");

    run_multi_thread(config, start);
}
