use anyhow::{Result, anyhow};
use tokio_rusqlite::{
    Connection,
    rusqlite::{self, Row},
};

use crate::Client;
use crate::{Config, now};

#[derive(Debug, Clone)]
pub struct Database {
    conn: Connection,
}

#[derive(Debug)]
struct ClientRow {
    id: String,
    owner_id: Option<String>,
    client_type: String,
    public_key: String,
    created_at: i64,
    last_seen: i64,
}

impl Database {
    pub const RELAY_API_KEY_CONFIG_KEY: &'static str = "relay_api_key";
    pub const NOISE_STATIC_PRIVATE_KEY_CONFIG_KEY: &'static str = "noise_static_private_key";

    pub async fn new(config: &Config) -> Result<Self> {
        let conn = if config.db_name == ":memory:" {
            Connection::open_in_memory().await?
        } else {
            Connection::open(config.dir_root.join(&config.db_name)).await?
        };

        Ok(Self { conn })
    }

    pub async fn init(&self) -> Result<()> {
        self.conn
            .call(|conn| -> rusqlite::Result<()> {
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
            })
            .await?;

        Ok(())
    }

    // Config
    pub async fn get_config(&self, key: &str) -> Result<Option<Vec<u8>>> {
        let key = key.to_string();
        let value = self
            .conn
            .call(move |conn| -> rusqlite::Result<Option<Vec<u8>>> {
                let mut stmt = conn.prepare("SELECT value FROM config WHERE key = ?1")?;
                let mut rows = stmt.query([key])?;
                Ok(rows.next()?.map(|row| row.get(0)).transpose()?)
            })
            .await?;
        Ok(value)
    }

    pub async fn set_config(&self, key: &str, value: &[u8]) -> Result<()> {
        let key = key.to_string();
        let value = value.to_vec();
        self.conn
            .call(move |conn| -> rusqlite::Result<()> {
                conn.execute(
                    "INSERT OR REPLACE INTO config (key, value) VALUES (?1, ?2)",
                    rusqlite::params![key, value],
                )?;
                Ok(())
            })
            .await?;
        Ok(())
    }

    pub async fn validate_relay_api_key(&self, api_key: &str) -> Result<()> {
        let Some(expected_api_key) = self.get_relay_api_key().await? else {
            return Err(anyhow!("invalid api key"));
        };

        if api_key == expected_api_key {
            return Ok(());
        }

        Err(anyhow!("invalid api key"))
    }

    pub async fn get_relay_api_key(&self) -> Result<Option<String>> {
        let Some(raw_value) = self.get_config(Self::RELAY_API_KEY_CONFIG_KEY).await? else {
            return Ok(None);
        };

        let value = String::from_utf8(raw_value)
            .map_err(|_| anyhow!("Invalid UTF-8 value for relay_api_key"))?;
        Ok(Some(value))
    }

    pub async fn set_relay_api_key(&self, api_key: &str) -> Result<()> {
        self.set_config(Self::RELAY_API_KEY_CONFIG_KEY, api_key.as_bytes())
            .await
    }

    pub async fn get_noise_static_private_key(&self) -> Result<Option<String>> {
        let Some(raw_value) = self
            .get_config(Self::NOISE_STATIC_PRIVATE_KEY_CONFIG_KEY)
            .await?
        else {
            return Ok(None);
        };

        let value = String::from_utf8(raw_value)
            .map_err(|_| anyhow!("Invalid UTF-8 value for noise_static_private_key"))?;
        Ok(Some(value))
    }

    pub async fn set_noise_static_private_key(&self, private_key: &str) -> Result<()> {
        self.set_config(
            Self::NOISE_STATIC_PRIVATE_KEY_CONFIG_KEY,
            private_key.as_bytes(),
        )
        .await
    }

    // Clients
    fn row_to_client_row(row: &Row<'_>) -> rusqlite::Result<ClientRow> {
        Ok(ClientRow {
            id: row.get(0)?,
            owner_id: row.get(1)?,
            client_type: row.get(2)?,
            public_key: row.get(3)?,
            created_at: row.get(4)?,
            last_seen: row.get(5)?,
        })
    }

    fn client_from_row(row: ClientRow) -> Result<Client> {
        Ok(Client {
            id: row.id,
            owner_id: row.owner_id,
            client_type: row.client_type.parse()?,
            public_key: row.public_key,
            created_at: row.created_at,
            last_seen: row.last_seen,
        })
    }

    pub async fn insert_client(&self, client: &Client) -> Result<()> {
        let client = client.clone();
        self.conn
            .call(move |conn| -> rusqlite::Result<()> {
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
            })
            .await?;
        Ok(())
    }

    pub async fn get_client(&self, id: &str) -> Result<Option<Client>> {
        let id = id.to_string();
        let row = self
            .conn
            .call(move |conn| -> rusqlite::Result<Option<ClientRow>> {
                let mut stmt = conn.prepare(
                    "SELECT id, owner_id, client_type, public_key, created_at, last_seen
                    FROM clients WHERE id = ?1",
                )?;
                let mut rows = stmt.query([id])?;
                let Some(row) = rows.next()? else {
                    return Ok(None);
                };
                Ok(Some(Self::row_to_client_row(row)?))
            })
            .await?;

        row.map(Self::client_from_row).transpose()
    }

    pub async fn get_client_by_public_key(&self, public_key: &str) -> Result<Option<Client>> {
        let public_key = public_key.to_string();
        let row = self
            .conn
            .call(move |conn| -> rusqlite::Result<Option<ClientRow>> {
                let mut stmt = conn.prepare(
                    "SELECT id, owner_id, client_type, public_key, created_at, last_seen
                    FROM clients WHERE public_key = ?1",
                )?;
                let mut rows = stmt.query([public_key])?;
                let Some(row) = rows.next()? else {
                    return Ok(None);
                };
                Ok(Some(Self::row_to_client_row(row)?))
            })
            .await?;

        row.map(Self::client_from_row).transpose()
    }

    pub async fn get_client_with_children(
        &self,
        id: &str,
    ) -> Result<Option<(Client, Vec<Client>)>> {
        let id = id.to_string();
        let (parent_row, child_rows) = self
            .conn
            .call(
                move |conn| -> rusqlite::Result<(Option<ClientRow>, Vec<ClientRow>)> {
                    let mut stmt = conn.prepare(
                        "SELECT id, owner_id, client_type, public_key, created_at, last_seen
                    FROM clients
                    WHERE id = ?1 OR owner_id = ?1
                    ORDER BY CASE WHEN id = ?1 THEN 0 ELSE 1 END, created_at ASC",
                    )?;
                    let mut rows = stmt.query([&id])?;

                    let mut parent: Option<ClientRow> = None;
                    let mut children = Vec::new();

                    while let Some(row) = rows.next()? {
                        let client = Self::row_to_client_row(row)?;
                        if client.id == id {
                            parent = Some(client);
                        } else {
                            children.push(client);
                        }
                    }

                    Ok((parent, children))
                },
            )
            .await?;

        let Some(parent) = parent_row else {
            return Ok(None);
        };

        let parent = Self::client_from_row(parent)?;
        let children = child_rows
            .into_iter()
            .map(Self::client_from_row)
            .collect::<Result<Vec<_>>>()?;

        Ok(Some((parent, children)))
    }

    pub async fn list_children(&self, owner_id: &str) -> Result<Vec<Client>> {
        let owner_id = owner_id.to_string();
        let rows = self
            .conn
            .call(move |conn| -> rusqlite::Result<Vec<ClientRow>> {
                let mut stmt = conn.prepare(
                    "SELECT id, owner_id, client_type, public_key, created_at, last_seen
                    FROM clients WHERE owner_id = ?1 ORDER BY created_at ASC",
                )?;
                let mut rows = stmt.query([owner_id])?;
                let mut out = Vec::new();

                while let Some(row) = rows.next()? {
                    out.push(Self::row_to_client_row(row)?);
                }

                Ok(out)
            })
            .await?;

        rows.into_iter()
            .map(Self::client_from_row)
            .collect::<Result<Vec<_>>>()
    }

    pub async fn touch_client(&self, id: &str) -> Result<()> {
        let id = id.to_string();
        self.conn
            .call(move |conn| -> rusqlite::Result<()> {
                conn.execute(
                    "UPDATE clients SET last_seen = ?1 WHERE id = ?2",
                    rusqlite::params![now(), id],
                )?;
                Ok(())
            })
            .await?;
        Ok(())
    }

    pub async fn delete_client(&self, id: &str) -> Result<()> {
        let id = id.to_string();
        self.conn
            .call(move |conn| -> rusqlite::Result<()> {
                conn.execute("DELETE FROM clients WHERE id = ?1", [id])?;
                Ok(())
            })
            .await?;
        Ok(())
    }
}
