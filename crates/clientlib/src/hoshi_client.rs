use anyhow::Result;

use crate::{Database, HoshiNetClient};

pub struct HoshiClient {
    pub net: HoshiNetClient,

    db: Database,
}

impl HoshiClient {
    pub async fn new() -> Result<Self> {
        let net = HoshiNetClient::new();
        let path = dirs::home_dir().unwrap().join(".hoshi");
        std::fs::create_dir_all(&path)?;
        let path = path.join("client.sqlite3");
        
        let db = Database::new(path).await?;
        db.init().await?;

        Ok(Self {
            net,
            db,
        })
    }
}

