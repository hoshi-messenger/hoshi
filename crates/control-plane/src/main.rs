use clap::Parser;
use hoshi_control_plane::{Config, run_multi_thread};
use std::path::PathBuf;

#[derive(Parser)]
struct Args {
    /// Path to the SQLite database file
    #[arg(long)]
    db_path: Option<PathBuf>,
}

fn main() {
    let start = std::time::Instant::now();
    let args = Args::parse();
    let mut config = Config::new();
    if let Some(path) = args.db_path {
        let dir = path.parent().unwrap_or(&path).to_str().unwrap();
        let name = path.file_name().unwrap().to_str().unwrap();
        config = config.set_dir_root(dir).set_db_name(name);
    }
    run_multi_thread(config, start);
}
