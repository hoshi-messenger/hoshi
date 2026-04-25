use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{ContactType, Store};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct HoshiRecord {
    pub id: Uuid,
    pub from: String,
    pub payload: HoshiPayload,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum HoshiPayload {
    Config {
        key: String,
        value: String,
    },
    Contact {
        public_key: String,
        contact_type: ContactType,
    },
    Text {
        content: String,
    },
    Title {
        title: String,
    },
}

impl HoshiRecord {
    pub fn new(from: String, payload: HoshiPayload) -> Self {
        Self {
            id: Uuid::now_v7(),
            from,
            payload,
        }
    }

    pub fn with_id(id: Uuid, from: String, payload: HoshiPayload) -> Self {
        Self { id, from, payload }
    }
}

impl Store for HoshiRecord {
    fn id(&self) -> Uuid {
        self.id
    }

    fn hash(&self) -> blake3::Hash {
        let mut hasher = blake3::Hasher::new();
        hasher
            .update(self.id.as_bytes())
            .update(self.from.as_bytes());
        match &self.payload {
            HoshiPayload::Config { key, value } => {
                hasher
                    .update(b"config")
                    .update(key.as_bytes())
                    .update(value.as_bytes());
            }
            HoshiPayload::Contact {
                public_key,
                contact_type,
            } => {
                hasher
                    .update(b"contact")
                    .update(public_key.as_bytes())
                    .update(format!("{contact_type:?}").as_bytes());
            }
            HoshiPayload::Text { content } => {
                hasher.update(b"text").update(content.as_bytes());
            }
            HoshiPayload::Title { title } => {
                hasher.update(b"title").update(title.as_bytes());
            }
        }
        hasher.finalize()
    }
}
