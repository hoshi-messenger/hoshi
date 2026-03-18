use std::{
    path::PathBuf,
    sync::mpsc::{self, TryRecvError},
    thread::{self, JoinHandle},
};

use anyhow::Result;
use rusqlite::{Connection, OptionalExtension};

use crate::Contact;

#[derive(Debug, Clone)]
enum DBMessage {
    Kill,

    ContactsGet,
    ContactUpsert(Contact),
    ContactDelete { public_key: String },

    ConfigGet(String),
    ConfigSet { key: String, value: Vec<u8> },
}

#[derive(Debug, Clone)]
pub enum DBReply {
    Shutdown,
    Contacts(Vec<Contact>),
    Config(Option<Vec<u8>>),
}

#[derive(Debug)]
pub struct Database {
    tx: mpsc::Sender<DBMessage>,
    rx: mpsc::Receiver<DBReply>,
    thread: Option<JoinHandle<()>>,
}

impl Database {
    pub fn new(path: PathBuf) -> Result<Self> {
        let (db_tx, db_rx) = mpsc::channel::<DBReply>();
        let (cli_tx, cli_rx) = mpsc::channel::<DBMessage>();
        let conn = Connection::open(path)?;

        // Run the actual DB on a separate thread, this is since SQLite does blocking
        // IO calls and depending on the query it might take a couple of ms, so better
        // to run it all on a separate thread, another side benefit is that due to the
        // message format we don't couple the clientlib/client to a DB implementation.
        let thread = thread::spawn(move || {
            let rx = cli_rx;
            let tx = db_tx;

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
            )
            .expect("Couldn't init DB");

            loop {
                if let Ok(msg) = rx.recv() {
                    match msg {
                        DBMessage::Kill => {
                            tx.send(DBReply::Shutdown)
                                .expect("DB couldn't send shutdown msg to client");
                            break;
                        }
                        DBMessage::ContactDelete { public_key } => {
                            conn.execute(
                                "DELETE from contact WHERE public_key = ?1",
                                rusqlite::params![&public_key],
                            )
                            .expect("Error deleting contact");
                        }
                        DBMessage::ContactUpsert(contact) => {
                            conn.execute(
                                "INSERT INTO contact (public_key, alias, created_at, last_seen)
                                VALUES (?1, ?2, unixepoch(), unixepoch())
                                ON CONFLICT(public_key) DO UPDATE SET
                                    alias = excluded.alias,
                                    last_seen = unixepoch()",
                                rusqlite::params![contact.public_key, contact.alias],
                            )
                            .expect("Error upserting contact");
                        }
                        DBMessage::ContactsGet => {
                            let mut stmt = conn
                                .prepare("SELECT public_key, alias FROM contact")
                                .expect("Error preparing contacts query");
                            let contacts = stmt
                                .query_map([], |row| {
                                    Ok(Contact {
                                        public_key: row.get(0)?,
                                        alias: row.get(1)?,
                                    })
                                })
                                .expect("Error querying contacts")
                                .filter_map(|r| r.ok())
                                .collect();

                            tx.send(DBReply::Contacts(contacts))
                                .expect("DB couldn't send contacts to client");
                        }
                        DBMessage::ConfigGet(key) => {
                            let val: Option<Vec<u8>> = conn
                                .query_row(
                                    "SELECT value FROM config WHERE key = ?1",
                                    rusqlite::params![key],
                                    |row| row.get(0),
                                )
                                .optional()
                                .expect("Error querying config");
                            tx.send(DBReply::Config(val))
                                .expect("DB couldn't send config reply");
                        }
                        DBMessage::ConfigSet { key, value } => {
                            conn.execute(
                                "INSERT OR REPLACE INTO config (key, value) VALUES (?1, ?2)",
                                rusqlite::params![key, value],
                            )
                            .expect("Error setting config");
                        }
                    }
                }
            }
        });

        Ok(Self {
            tx: cli_tx,
            rx: db_rx,
            thread: Some(thread),
        })
    }

    pub fn contacts_get(&self) -> Result<()> {
        self.tx.send(DBMessage::ContactsGet)?;
        Ok(())
    }

    pub fn contact_upsert(&self, contact: Contact) -> Result<()> {
        self.tx.send(DBMessage::ContactUpsert(contact))?;
        Ok(())
    }

    pub fn contact_delete(&self, public_key: String) -> Result<()> {
        self.tx.send(DBMessage::ContactDelete { public_key })?;
        Ok(())
    }

    /// Blocking config read — only safe to call at startup before any other messages are queued.
    pub fn config_get_blocking(&self, key: &str) -> Option<Vec<u8>> {
        self.tx.send(DBMessage::ConfigGet(key.to_string())).unwrap();
        match self.rx.recv().unwrap() {
            DBReply::Config(val) => val,
            other => panic!(
                "Unexpected DB reply during config_get_blocking: {:?}",
                other
            ),
        }
    }

    pub fn config_set(&self, key: &str, value: Vec<u8>) -> Result<()> {
        self.tx.send(DBMessage::ConfigSet {
            key: key.to_string(),
            value,
        })?;
        Ok(())
    }

    pub fn recv(&self) -> Option<DBReply> {
        match self.rx.try_recv() {
            Err(TryRecvError::Empty) => None,
            Ok(msg) => Some(msg),
            _ => {
                panic!("Thread disconnected while connectoin was running");
            }
        }
    }
}

impl Drop for Database {
    fn drop(&mut self) {
        self.tx
            .send(DBMessage::Kill)
            .expect("Couldn't send kill to DB thread");
        if let Some(thread) = self.thread.take() {
            thread.join().expect("Couldn't join DB thread");
        }
    }
}
