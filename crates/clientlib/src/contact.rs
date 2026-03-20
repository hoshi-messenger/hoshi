use serde::{Deserialize, Serialize};

use crate::NodeStore;

fn generate_emoji_alias(public_key: &str) -> String {
    const EMOJIS: &[&str] = &[
        "🐶", "🐱", "🐭", "🐹", "🐰", "🦊", "🐻", "🐼", "🐨", "🐯", "🦁", "🐮", "🐷", "🐸", "🐵",
        "🐔", "🐧", "🐦", "🦆", "🦉", "🦇", "🐺", "🐗", "🐴", "🦄", "🐝", "🐛", "🦋", "🐌", "🐞",
        "🦎", "🐍", "🐢", "🦖", "🦕", "🐙", "🦑", "🦐", "🦀", "🐡", "🐠", "🐟", "🐬", "🐳", "🦈",
        "🐊", "🦧", "🦥", "🦦", "🦔",
    ];

    let hash: u64 = public_key.bytes().fold(0xcbf29ce484222325u64, |acc, b| {
        acc.wrapping_mul(0x100000001b3).wrapping_add(b as u64) // FNV-1a
    });

    let first = EMOJIS[(hash % EMOJIS.len() as u64) as usize];
    let second = EMOJIS[((hash >> 8) % EMOJIS.len() as u64) as usize];

    format!("{}{}", first, second)
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum ContactType {
    Unknown,
    Contact,
    Blocked,
    Deleted,
}

#[derive(Debug, Clone)]
pub struct Contact {
    pub public_key: String,
    pub contact_type: ContactType,
}

impl Contact {
    pub fn new(public_key: String) -> Contact {
        Self {
            public_key,
            contact_type: ContactType::Contact,
        }
    }

    pub fn new_unknown(public_key: String) -> Contact {
        Self {
            public_key,
            contact_type: ContactType::Unknown,
        }
    }

    /// Returns the user-advertised title if available via the node store,
    /// otherwise falls back to a deterministic emoji alias.
    pub fn display_name(&self, nodes: Option<&mut NodeStore>) -> String {
        if let Some(nodes) = nodes {
            if let Some(title) = nodes.user_alias_get(&self.public_key) {
                return title;
            }
        }
        generate_emoji_alias(&self.public_key)
    }
}
