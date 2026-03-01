use std::{
    fs,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use anyhow::{Context, Result, anyhow, bail};
use hoshi_protocol::control_plane::ClientType;
use tokio_rusqlite::{
    Connection,
    rusqlite::{self, Row},
};
use uuid::Uuid;

use crate::noise::canonicalize_base64_32;

const CONTROL_PLANE_URI_CONFIG_KEY: &str = "control_plane_uri";
const DEVICE_GUID_CONFIG_KEY: &str = "device_guid";
const USER_GUID_CONFIG_KEY: &str = "user_guid";

#[derive(Debug, Clone)]
pub struct ClientDatabase {
    pub db_path: PathBuf,
    conn: Connection,
}

#[derive(Debug, Clone)]
pub struct StoredKey {
    pub guid: String,
    pub client_type: ClientType,
    pub private_key: String,
    pub created_at: i64,
    pub last_used: i64,
}

#[derive(Debug)]
struct StoredKeyRow {
    guid: String,
    client_type: String,
    private_key: String,
    created_at: i64,
    last_used: i64,
}

impl ClientDatabase {
    pub async fn open_default() -> Result<Self> {
        Self::open(default_db_path()).await
    }

    pub async fn open(path: impl AsRef<Path>) -> Result<Self> {
        let db_path = path.as_ref().to_path_buf();
        let conn = if db_path == Path::new(":memory:") {
            Connection::open_in_memory().await?
        } else {
            if let Some(parent) = db_path.parent() {
                fs::create_dir_all(parent).with_context(|| {
                    format!(
                        "failed to create client database directory {}",
                        parent.display()
                    )
                })?;
            }
            Connection::open(&db_path).await?
        };

        let db = Self { conn, db_path };
        db.init().await?;
        db.ensure_defaults().await?;
        Ok(db)
    }

    async fn init(&self) -> Result<()> {
        self.conn
            .call(|conn| -> rusqlite::Result<()> {
                conn.execute_batch(
                    "
                    PRAGMA journal_mode=WAL;

                    CREATE TABLE IF NOT EXISTS keys (
                        guid TEXT NOT NULL,
                        type TEXT NOT NULL,
                        private_key TEXT NOT NULL,
                        created_at INTEGER NOT NULL,
                        last_used INTEGER NOT NULL,
                        PRIMARY KEY (guid, type)
                    );

                    CREATE TABLE IF NOT EXISTS config (
                        key TEXT PRIMARY KEY,
                        value BLOB NOT NULL
                    );

                    CREATE INDEX IF NOT EXISTS idx_keys_last_used
                    ON keys(last_used);
                ",
                )?;
                Ok(())
            })
            .await?;
        Ok(())
    }

    async fn ensure_defaults(&self) -> Result<()> {
        if self
            .get_config_string(CONTROL_PLANE_URI_CONFIG_KEY)
            .await?
            .is_none()
        {
            self.set_config_string(CONTROL_PLANE_URI_CONFIG_KEY, &default_control_plane_uri())
                .await?;
        }
        Ok(())
    }

    pub async fn get_config(&self, key: &str) -> Result<Option<Vec<u8>>> {
        let key = key.to_string();
        let value = self
            .conn
            .call(move |conn| -> rusqlite::Result<Option<Vec<u8>>> {
                let mut stmt = conn.prepare("SELECT value FROM config WHERE key = ?1")?;
                let mut rows = stmt.query([key])?;
                rows.next()?.map(|row| row.get(0)).transpose()
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

    async fn delete_config(&self, key: &str) -> Result<()> {
        let key = key.to_string();
        self.conn
            .call(move |conn| -> rusqlite::Result<()> {
                conn.execute("DELETE FROM config WHERE key = ?1", [key])?;
                Ok(())
            })
            .await?;
        Ok(())
    }

    pub async fn get_config_string(&self, key: &str) -> Result<Option<String>> {
        let Some(raw_value) = self.get_config(key).await? else {
            return Ok(None);
        };

        let value =
            String::from_utf8(raw_value).map_err(|_| anyhow!("invalid UTF-8 value for {key}"))?;
        Ok(Some(value))
    }

    pub async fn set_config_string(&self, key: &str, value: &str) -> Result<()> {
        self.set_config(key, value.as_bytes()).await
    }

    pub async fn get_control_plane_uri(&self) -> Result<String> {
        let value = self
            .get_config_string(CONTROL_PLANE_URI_CONFIG_KEY)
            .await?
            .unwrap_or_else(default_control_plane_uri);

        let value = value.trim();
        if value.is_empty() {
            return Ok(default_control_plane_uri());
        }

        Ok(value.to_string())
    }

    pub async fn set_control_plane_uri(&self, uri: &str) -> Result<()> {
        let uri = uri.trim();
        if uri.is_empty() {
            bail!("control_plane_uri must not be empty");
        }
        self.set_config_string(CONTROL_PLANE_URI_CONFIG_KEY, uri).await
    }

    pub async fn get_device_guid(&self) -> Result<Option<String>> {
        self.get_guid_config(DEVICE_GUID_CONFIG_KEY).await
    }

    pub async fn set_device_guid(&self, guid: &str) -> Result<()> {
        self.set_guid_config(DEVICE_GUID_CONFIG_KEY, guid).await
    }

    pub async fn clear_device_guid(&self) -> Result<()> {
        self.delete_config(DEVICE_GUID_CONFIG_KEY).await
    }

    pub async fn get_user_guid(&self) -> Result<Option<String>> {
        self.get_guid_config(USER_GUID_CONFIG_KEY).await
    }

    pub async fn set_user_guid(&self, guid: &str) -> Result<()> {
        self.set_guid_config(USER_GUID_CONFIG_KEY, guid).await
    }

    pub async fn clear_user_guid(&self) -> Result<()> {
        self.delete_config(USER_GUID_CONFIG_KEY).await
    }

    async fn get_guid_config(&self, key: &str) -> Result<Option<String>> {
        let Some(value) = self.get_config_string(key).await? else {
            return Ok(None);
        };
        if value.trim().is_empty() {
            return Ok(None);
        }
        Ok(Some(canonicalize_guid(&value)?))
    }

    async fn set_guid_config(&self, key: &str, guid: &str) -> Result<()> {
        let canonical_guid = canonicalize_guid(guid)?;
        self.set_config_string(key, &canonical_guid).await
    }

    pub async fn upsert_key(
        &self,
        guid: &str,
        client_type: ClientType,
        private_key_b64: &str,
    ) -> Result<()> {
        let guid = canonicalize_guid(guid)?;
        let private_key = canonicalize_base64_32(private_key_b64.trim(), "private_key")?;
        let client_type = client_type_to_db_value(&client_type).to_string();
        let timestamp = now();

        self.conn
            .call(move |conn| -> rusqlite::Result<()> {
                conn.execute(
                    "INSERT OR REPLACE INTO keys (guid, type, private_key, created_at, last_used)
                     VALUES (?1, ?2, ?3, ?4, ?5)",
                    rusqlite::params![guid, client_type, private_key, timestamp, timestamp],
                )?;
                Ok(())
            })
            .await?;
        Ok(())
    }

    pub async fn get_key(&self, guid: &str, client_type: ClientType) -> Result<Option<StoredKey>> {
        let guid = canonicalize_guid(guid)?;
        let client_type = client_type_to_db_value(&client_type).to_string();

        let row = self
            .conn
            .call(move |conn| -> rusqlite::Result<Option<StoredKeyRow>> {
                let mut stmt = conn.prepare(
                    "SELECT guid, type, private_key, created_at, last_used
                     FROM keys WHERE guid = ?1 AND type = ?2",
                )?;
                let mut rows = stmt.query(rusqlite::params![guid, client_type])?;
                let Some(row) = rows.next()? else {
                    return Ok(None);
                };
                Ok(Some(Self::row_to_stored_key_row(row)?))
            })
            .await?;

        row.map(Self::stored_key_from_row).transpose()
    }

    pub async fn list_keys(&self) -> Result<Vec<StoredKey>> {
        let rows = self
            .conn
            .call(|conn| -> rusqlite::Result<Vec<StoredKeyRow>> {
                let mut stmt = conn.prepare(
                    "SELECT guid, type, private_key, created_at, last_used
                     FROM keys ORDER BY guid, type",
                )?;
                let iter = stmt.query_map([], Self::row_to_stored_key_row)?;
                iter.collect()
            })
            .await?;

        rows.into_iter().map(Self::stored_key_from_row).collect()
    }

    pub async fn touch_key(&self, guid: &str, client_type: ClientType) -> Result<()> {
        let guid = canonicalize_guid(guid)?;
        let client_type = client_type_to_db_value(&client_type).to_string();
        let timestamp = now();

        self.conn
            .call(move |conn| -> rusqlite::Result<()> {
                conn.execute(
                    "UPDATE keys SET last_used = ?1 WHERE guid = ?2 AND type = ?3",
                    rusqlite::params![timestamp, guid, client_type],
                )?;
                Ok(())
            })
            .await?;
        Ok(())
    }

    fn row_to_stored_key_row(row: &Row<'_>) -> rusqlite::Result<StoredKeyRow> {
        Ok(StoredKeyRow {
            guid: row.get(0)?,
            client_type: row.get(1)?,
            private_key: row.get(2)?,
            created_at: row.get(3)?,
            last_used: row.get(4)?,
        })
    }

    fn stored_key_from_row(row: StoredKeyRow) -> Result<StoredKey> {
        Ok(StoredKey {
            guid: canonicalize_guid(&row.guid)?,
            client_type: client_type_from_db_value(&row.client_type)?,
            private_key: canonicalize_base64_32(&row.private_key, "private_key")?,
            created_at: row.created_at,
            last_used: row.last_used,
        })
    }
}

fn default_db_path() -> PathBuf {
    dirs::home_dir()
        .map(|home| home.join(".hoshi").join("client.sqlite3"))
        .unwrap_or_else(|| PathBuf::from("./.hoshi/client.sqlite3"))
}

fn default_control_plane_uri() -> String {
    if cfg!(debug_assertions) {
        "http://127.0.0.1:2600".to_string()
    } else {
        "https://cp.wikinarau.org".to_string()
    }
}

fn now() -> i64 {
    let duration = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time before unix epoch");
    i64::try_from(duration.as_secs()).expect("timestamp overflow")
}

fn canonicalize_guid(guid: &str) -> Result<String> {
    Ok(Uuid::parse_str(guid.trim()).context("invalid guid")?.to_string())
}

fn client_type_to_db_value(client_type: &ClientType) -> &'static str {
    match client_type {
        ClientType::Device => "device",
        ClientType::User => "user",
        ClientType::Relay => "relay",
    }
}

fn client_type_from_db_value(client_type: &str) -> Result<ClientType> {
    match client_type {
        "device" => Ok(ClientType::Device),
        "user" => Ok(ClientType::User),
        "relay" => Ok(ClientType::Relay),
        other => Err(anyhow!("invalid client type in keys table: {other}")),
    }
}
