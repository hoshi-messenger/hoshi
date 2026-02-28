use std::{sync::{Arc, Mutex}};

use anyhow::{Result, anyhow};
use rusqlite::Connection;

use crate::{Config, now};
use crate::Client;

#[derive(Debug, Clone)]
pub struct Database {
    conn: Arc<Mutex<Connection>>,
}

impl Database {
    pub fn new(config: &Config) -> Result<Self> {
        let db = if config.db_name == ":memory:" {
            Connection::open_in_memory()
        } else {
            Connection::open(config.dir_root.join(&config.db_name))
        }?;
        let conn = Arc::new(Mutex::new(db));

        Ok(Self {
            conn,
        })
    }

    pub fn init(&self) -> Result<()> {
        let conn = self.conn.lock().map_err(|_| anyhow!("Couldn't lock DB for init"))?;

        conn.execute_batch("
            PRAGMA journal_mode=WAL;
            
            CREATE TABLE IF NOT EXISTS clients (
                id TEXT PRIMARY KEY,
                owner_id TEXT REFERENCES clients(id),
                client_type TEXT NOT NULL,
                public_key BLOB NOT NULL,
                created_at INTEGER NOT NULL,
                last_seen INTEGER NOT NULL
            );

            CREATE TABLE IF NOT EXISTS config (
                key TEXT PRIMARY KEY,
                value BLOB NOT NULL
            );
        ")?;

        Ok(())
    }

    // Config
    pub fn get_config(&self, key: &str) -> Result<Option<Vec<u8>>> {
        let conn = self.conn.lock().map_err(|_| anyhow!("Couldn't lock DB"))?;
        let mut stmt = conn.prepare("SELECT value FROM config WHERE key = ?1")?;
        let mut rows = stmt.query([key])?;
        Ok(rows.next()?.map(|row| row.get(0)).transpose()?)
    }

    pub fn set_config(&self, key: &str, value: &[u8]) -> Result<()> {
        let conn = self.conn.lock().map_err(|_| anyhow!("Couldn't lock DB"))?;
        conn.execute(
            "INSERT OR REPLACE INTO config (key, value) VALUES (?1, ?2)",
            rusqlite::params![key, value],
        )?;
        Ok(())
    }

    // Clients
    pub fn insert_client(&self, client: &Client) -> Result<()> {
        let conn = self.conn.lock().map_err(|_| anyhow!("Couldn't lock DB"))?;
        conn.execute(
            "INSERT INTO clients (id, owner_id, client_type, public_key, created_at, last_seen)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            rusqlite::params![
                client.id,
                client.owner_id,
                client.client_type.to_string(),
                client.public_key,
                client.created_at,
                client.last_seen,
            ],
        )?;
        Ok(())
    }

    pub fn get_client(&self, id: &str) -> Result<Option<Client>> {
        let conn = self.conn.lock().map_err(|_| anyhow!("Couldn't lock DB"))?;
        let mut stmt = conn.prepare(
            "SELECT id, owner_id, client_type, public_key, created_at, last_seen
            FROM clients WHERE id = ?1"
        )?;
        let mut rows = stmt.query([id])?;
        let Some(row) = rows.next()? else { return Ok(None) };
        let client_type_str: String = row.get(2)?;
        let owner_id: Option<String> = row.get(1)?;
        Ok(Some(Client {
            id: row.get(0)?,
            owner_id,
            client_type: client_type_str.parse()?,
            public_key: row.get(3)?,
            created_at: row.get(4)?,
            last_seen: row.get(5)?,
        }))
    }

    pub fn touch_client(&self, id: &str) -> Result<()> {
        let conn = self.conn.lock().map_err(|_| anyhow!("Couldn't lock DB"))?;
        conn.execute(
            "UPDATE clients SET last_seen = ?1 WHERE id = ?2",
            rusqlite::params![now(), id],
        )?;
        Ok(())
    }

    pub fn delete_client(&self, id: &str) -> Result<()> {
        let conn = self.conn.lock().map_err(|_| anyhow!("Couldn't lock DB"))?;
        conn.execute("DELETE FROM clients WHERE id = ?1", [id])?;
        Ok(())
    }
}