use std::{
    path::PathBuf,
    sync::mpsc::{self, TryRecvError},
    thread::{self, JoinHandle},
};

use anyhow::Result;
use rusqlite::Connection;

use crate::Contact;

#[derive(Debug, Clone)]
enum DBMessage {
    Ping,
    Kill,
}

#[derive(Debug, Clone)]
pub enum DBReply {
    Pong,
    Shutdown,
    UpsertContact(Contact),
    DeleteContact{public_key: String}
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
                        DBMessage::Ping => {
                            tx.send(DBReply::Pong)
                                .expect("DB couldn't send pong msg to client");
                        }
                        DBMessage::Kill => {
                            tx.send(DBReply::Shutdown)
                                .expect("DB couldn't send shutdown msg to client");
                            break;
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

    pub fn ping(&self) -> Result<()> {
        self.tx.send(DBMessage::Ping)?;
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
