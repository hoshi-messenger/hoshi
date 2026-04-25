use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{ContactType, Store, identity::HoshiIdentity};

const RECORD_SIGNATURE_DOMAIN: &[u8] = b"hoshi-record-v1";

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct HoshiRecord {
    pub id: Uuid,
    pub from: String,
    pub payload: HoshiPayload,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct HoshiSignedRecord {
    pub record: HoshiRecord,
    pub signature: Vec<u8>,
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

impl HoshiSignedRecord {
    pub fn sign(record: HoshiRecord, identity: &HoshiIdentity) -> anyhow::Result<Self> {
        let bytes = signing_bytes(&record)?;
        Ok(Self {
            record,
            signature: identity.sign(&bytes).to_vec(),
        })
    }

    pub fn verify(&self) -> bool {
        let Ok(bytes) = signing_bytes(&self.record) else {
            return false;
        };
        let Ok(signature) = self.signature.as_slice().try_into() else {
            return false;
        };
        HoshiIdentity::verify(&self.record.from, &bytes, signature)
    }
}

fn signing_bytes(record: &HoshiRecord) -> anyhow::Result<Vec<u8>> {
    let mut bytes = Vec::from(RECORD_SIGNATURE_DOMAIN);
    bytes.extend_from_slice(&rmp_serde::to_vec(record)?);
    Ok(bytes)
}

impl Store for HoshiSignedRecord {
    fn id(&self) -> Uuid {
        self.record.id
    }

    fn hash(&self) -> blake3::Hash {
        let mut hasher = blake3::Hasher::new();
        hasher
            .update(self.record.id.as_bytes())
            .update(self.record.from.as_bytes());
        match &self.record.payload {
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
        hasher.update(&self.signature);
        hasher.finalize()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::identity::HoshiIdentity;

    #[test]
    fn signed_record_verifies() -> anyhow::Result<()> {
        let identity = HoshiIdentity::generate();
        let record = HoshiRecord::new(
            identity.public_key_hex(),
            HoshiPayload::Text {
                content: "hello".to_string(),
            },
        );

        let signed = HoshiSignedRecord::sign(record, &identity)?;

        assert!(signed.verify());
        Ok(())
    }

    #[test]
    fn tampered_signed_record_fails_verification() -> anyhow::Result<()> {
        let identity = HoshiIdentity::generate();
        let record = HoshiRecord::new(
            identity.public_key_hex(),
            HoshiPayload::Title {
                title: "Alice".to_string(),
            },
        );
        let mut signed = HoshiSignedRecord::sign(record, &identity)?;

        signed.record.payload = HoshiPayload::Title {
            title: "Mallory".to_string(),
        };

        assert!(!signed.verify());
        Ok(())
    }

    #[test]
    fn changed_author_fails_verification() -> anyhow::Result<()> {
        let identity = HoshiIdentity::generate();
        let other = HoshiIdentity::generate();
        let record = HoshiRecord::new(
            identity.public_key_hex(),
            HoshiPayload::Text {
                content: "hello".to_string(),
            },
        );
        let mut signed = HoshiSignedRecord::sign(record, &identity)?;

        signed.record.from = other.public_key_hex();

        assert!(!signed.verify());
        Ok(())
    }
}
