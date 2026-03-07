use clap::Parser;
use std::path::PathBuf;

#[derive(Parser, Clone)]
pub struct Args {
    /// Path to the SQLite database file
    #[arg(long)]
    pub db_path: Option<PathBuf>,
}
