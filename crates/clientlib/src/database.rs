use std::{
    path::PathBuf,
    sync::mpsc::{self, TryRecvError},
    thread::{self, JoinHandle},
};

use anyhow::Result;
use rusqlite::Connection;

use crate::{ChatMessage, Contact};

#[derive(Debug, Clone)]
enum DBMessage {
    Kill,

    ContactsGet,
    ContactUpsert(Contact),
    ContactDelete { public_key: String },

    MessagesGet,
    MessageUpsert(ChatMessage),
}

#[derive(Debug, Clone)]
pub enum DBReply {
    Shutdown,
    Contacts(Vec<Contact>),
    Messages(Vec<ChatMessage>),
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
        let thread =
            thread::spawn(move || {
                let rx = cli_rx;
                let tx = db_tx;

                conn.execute_batch(
                    "
                PRAGMA journal_mode=WAL;

                CREATE TABLE IF NOT EXISTS chat (
                    id TEXT PRIMARY KEY,
                    created_at INTEGER NOT NULL,
                    from_key TEXT NOT NULL,
                    to_key TEXT NOT NULL,
                    content TEXT NOT NULL
                );
                CREATE INDEX IF NOT EXISTS idx_chat_from_key ON chat(from_key);
                CREATE INDEX IF NOT EXISTS idx_chat_to_key ON chat(to_key);

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
                            DBMessage::MessagesGet => {
                                let mut stmt = conn.prepare(
                                "SELECT id, created_at, from_key, to_key, content FROM chat"
                                ).expect("Error preparing chat query");
                                let messages = stmt
                                    .query_map([], |row| {
                                        Ok(ChatMessage::new(
                                            row.get(0)?,
                                            row.get(1)?,
                                            row.get(2)?,
                                            row.get(3)?,
                                            row.get(4)?,
                                        ))
                                    })
                                    .expect("Error querying contacts")
                                    .filter_map(|r| r.ok())
                                    .collect();

                                tx.send(DBReply::Messages(messages))
                                    .expect("DB couldn't send chat messages to client");
                            }
                            DBMessage::MessageUpsert(msg) => {
                                conn.execute(
                                    "INSERT INTO chat (id, created_at, from_key, to_key, content)
                                VALUES (?1, ?2, ?3, ?4, ?5)
                                ON CONFLICT(id) DO UPDATE SET
                                    content = excluded.content",
                                    rusqlite::params![
                                        msg.id,
                                        msg.created_at,
                                        msg.from,
                                        msg.to,
                                        msg.content
                                    ],
                                )
                                .expect("Error upserting chat");
                            }
                        }
                    }
                }
                ()
            });

        Ok(Self {
            tx: cli_tx,
            rx: db_rx,
            thread: Some(thread),
        })
    }

    pub fn messages_get(&self) -> Result<()> {
        self.tx.send(DBMessage::MessagesGet)?;
        Ok(())
    }

    pub fn message_upsert(&self, msg: ChatMessage) -> Result<()> {
        self.tx.send(DBMessage::MessageUpsert(msg))?;
        Ok(())
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
