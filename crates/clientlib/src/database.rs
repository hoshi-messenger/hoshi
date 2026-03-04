use std::path::PathBuf;

use anyhow::Result;
use tokio_rusqlite::{Connection, rusqlite};

pub struct Database {
    conn: Connection,
}

impl Database {
    pub async fn new(path: PathBuf) -> Result<Self> {
        let conn = Connection::open(path).await?;
        Ok(Self {
            conn,
        })
    }

    pub async fn init(&self) -> Result<()> {
        self.conn.call(|conn| -> rusqlite::Result<()> {
            conn.execute_batch(
                "
                PRAGMA journal_mode=WAL;

                CREATE TABLE IF NOT EXISTS contact (
                    public_key TEXT PRIMARY KEY,
                    alias TEXT,
                    created_at INTEGER NOT NULL,
                    last_seen INTEGER NOT NULL
                );

                CREATE TABLE IF NOT EXISTS config (
                    key TEXT PRIMARY KEY,
                    value BLOB NOT NULL
                );
            ",
            )?;
            Ok(())
        }).await?;

        Ok(())
    }
}