use std::sync::{Arc, Mutex};

use anyhow::{Result, anyhow};
use rusqlite::{Connection, Row};

use crate::Client;
use crate::{Config, now};

#[derive(Debug, Clone)]
pub struct Database {
    conn: Arc<Mutex<Connection>>,
}

impl Database {
    pub const RELAY_API_KEY_CONFIG_KEY: &'static str = "relay_api_key";
    pub const NOISE_STATIC_PRIVATE_KEY_CONFIG_KEY: &'static str = "noise_static_private_key";

    pub fn new(config: &Config) -> Result<Self> {
        let db = if config.db_name == ":memory:" {
            Connection::open_in_memory()
        } else {
            Connection::open(config.dir_root.join(&config.db_name))
        }?;
        let conn = Arc::new(Mutex::new(db));

        Ok(Self { conn })
    }

    pub fn init(&self) -> Result<()> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| anyhow!("Couldn't lock DB for init"))?;

        conn.execute_batch(
            "
            PRAGMA journal_mode=WAL;
            
            CREATE TABLE IF NOT EXISTS clients (
                id TEXT PRIMARY KEY,
                owner_id TEXT REFERENCES clients(id),
                client_type TEXT NOT NULL,
                public_key TEXT NOT NULL,
                created_at INTEGER NOT NULL,
                last_seen INTEGER NOT NULL
            );

            CREATE TABLE IF NOT EXISTS config (
                key TEXT PRIMARY KEY,
                value BLOB NOT NULL
            );

            CREATE UNIQUE INDEX IF NOT EXISTS idx_clients_public_key
            ON clients(public_key);

            CREATE INDEX IF NOT EXISTS idx_clients_owner_id
            ON clients(owner_id);
        ",
        )?;

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

    pub fn validate_relay_api_key(&self, api_key: &str) -> Result<()> {
        let Some(expected_api_key) = self.get_relay_api_key()? else {
            return Err(anyhow!("invalid api key"));
        };

        if api_key == expected_api_key {
            return Ok(());
        }

        Err(anyhow!("invalid api key"))
    }

    pub fn get_relay_api_key(&self) -> Result<Option<String>> {
        let Some(raw_value) = self.get_config(Self::RELAY_API_KEY_CONFIG_KEY)? else {
            return Ok(None);
        };

        let value = String::from_utf8(raw_value)
            .map_err(|_| anyhow!("Invalid UTF-8 value for relay_api_key"))?;
        Ok(Some(value))
    }

    pub fn set_relay_api_key(&self, api_key: &str) -> Result<()> {
        self.set_config(Self::RELAY_API_KEY_CONFIG_KEY, api_key.as_bytes())
    }

    pub fn get_noise_static_private_key(&self) -> Result<Option<String>> {
        let Some(raw_value) = self.get_config(Self::NOISE_STATIC_PRIVATE_KEY_CONFIG_KEY)? else {
            return Ok(None);
        };

        let value = String::from_utf8(raw_value)
            .map_err(|_| anyhow!("Invalid UTF-8 value for noise_static_private_key"))?;
        Ok(Some(value))
    }

    pub fn set_noise_static_private_key(&self, private_key: &str) -> Result<()> {
        self.set_config(
            Self::NOISE_STATIC_PRIVATE_KEY_CONFIG_KEY,
            private_key.as_bytes(),
        )
    }

    // Clients
    fn row_to_client(row: &Row<'_>) -> Result<Client> {
        let client_type_str: String = row.get(2)?;
        let owner_id: Option<String> = row.get(1)?;
        Ok(Client {
            id: row.get(0)?,
            owner_id,
            client_type: client_type_str.parse()?,
            public_key: row.get(3)?,
            created_at: row.get(4)?,
            last_seen: row.get(5)?,
        })
    }

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
            FROM clients WHERE id = ?1",
        )?;
        let mut rows = stmt.query([id])?;
        let Some(row) = rows.next()? else {
            return Ok(None);
        };
        Ok(Some(Self::row_to_client(row)?))
    }

    pub fn get_client_by_public_key(&self, public_key: &str) -> Result<Option<Client>> {
        let conn = self.conn.lock().map_err(|_| anyhow!("Couldn't lock DB"))?;
        let mut stmt = conn.prepare(
            "SELECT id, owner_id, client_type, public_key, created_at, last_seen
            FROM clients WHERE public_key = ?1",
        )?;
        let mut rows = stmt.query([public_key])?;
        let Some(row) = rows.next()? else {
            return Ok(None);
        };
        Ok(Some(Self::row_to_client(row)?))
    }

    pub fn get_client_with_children(&self, id: &str) -> Result<Option<(Client, Vec<Client>)>> {
        let conn = self.conn.lock().map_err(|_| anyhow!("Couldn't lock DB"))?;
        let mut stmt = conn.prepare(
            "SELECT id, owner_id, client_type, public_key, created_at, last_seen
            FROM clients
            WHERE id = ?1 OR owner_id = ?1
            ORDER BY CASE WHEN id = ?1 THEN 0 ELSE 1 END, created_at ASC",
        )?;
        let mut rows = stmt.query([id])?;

        let mut parent: Option<Client> = None;
        let mut children = Vec::new();

        while let Some(row) = rows.next()? {
            let client = Self::row_to_client(row)?;
            if client.id == id {
                parent = Some(client);
            } else {
                children.push(client);
            }
        }

        match parent {
            Some(parent) => Ok(Some((parent, children))),
            None => Ok(None),
        }
    }

    pub fn list_children(&self, owner_id: &str) -> Result<Vec<Client>> {
        let conn = self.conn.lock().map_err(|_| anyhow!("Couldn't lock DB"))?;
        let mut stmt = conn.prepare(
            "SELECT id, owner_id, client_type, public_key, created_at, last_seen
            FROM clients WHERE owner_id = ?1 ORDER BY created_at ASC",
        )?;
        let mut rows = stmt.query([owner_id])?;
        let mut out = Vec::new();

        while let Some(row) = rows.next()? {
            out.push(Self::row_to_client(row)?);
        }

        Ok(out)
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
