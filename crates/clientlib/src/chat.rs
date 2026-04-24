use serde::{Deserialize, Serialize};

use crate::node;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ChatMessage {
    pub id: String,
    pub created_at: i64,
    pub from: String,
    pub to: String,
    pub content: String,
}

impl PartialEq for ChatMessage {
    fn eq(&self, other: &Self) -> bool {
        self.created_at == other.created_at && self.id == other.id
    }
}

impl Eq for ChatMessage {}

impl PartialOrd for ChatMessage {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for ChatMessage {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.created_at
            .cmp(&other.created_at)
            .then_with(|| self.id.cmp(&other.id))
    }
}

impl ChatMessage {
    pub fn new(id: String, created_at: i64, from: String, to: String, content: String) -> Self {
        Self {
            id,
            created_at,
            from,
            to,
            content,
        }
    }

    pub fn create(from: String, to: String, content: String) -> Self {
        let id = uuid::Uuid::now_v7().to_string();
        Self::new(id, 0, from, to, content)
    }

    pub fn chat_id(&self) -> String {
        Self::calc_chat_id(&self.from, &self.to)
    }

    pub fn calc_chat_id(from: &str, to: &str) -> String {
        node::chat_path(from, to)
    }
}
