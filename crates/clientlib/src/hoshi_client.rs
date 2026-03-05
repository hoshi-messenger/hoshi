use anyhow::Result;

use crate::{Database, HoshiNetClient, database::DBReply};

#[derive(Debug)]
pub struct HoshiClient {
    pub net: HoshiNetClient,

    db: Database,
}

impl HoshiClient {
    pub fn new() -> Result<Self> {
        let net = HoshiNetClient::new();
        let path = dirs::home_dir().unwrap().join(".hoshi");
        std::fs::create_dir_all(&path)?;
        let path = path.join("client.sqlite3");
        let db = Database::new(path)?;
        db.ping()?;

        Ok(Self {
            net,
            db,
        })
    }

    fn handle_db_msg(&self, msg: DBReply) {
        match msg {
            DBReply::Pong => {
                println!("Client/DB: Pong");
            },
            DBReply::Shutdown => {
                println!("Client/DB: Shutdown");
            }
        }
    }

    pub fn step(&self) -> u32 {
        let mut msgs = 0;

        // Only handle at most 32 msgs per iteration, make sure the
        // calling event loop doesn't block for too long, exact value
        // will have to be fine-tuned once we have an actual workload
        for _i in 1..32 {
            if let Some(msg) = self.db.recv() {
                msgs += 1;
                self.handle_db_msg(msg);
            } else {
                break;
            }
        }

        msgs
    }
}

