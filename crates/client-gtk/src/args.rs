use clap::Parser;
use std::path::PathBuf;

#[derive(Parser, Clone)]
pub struct Args {
    /// Path to the data directory
    #[arg(long)]
    pub data_dir: Option<PathBuf>,
}
