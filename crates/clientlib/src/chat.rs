use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::node::{self, HoshiNodePayload, NodeStore};

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

    /// Build a HashMap of ChatMessages from the NodeStore for a given chat path.
    /// Message nodes are direct children of chat_path, their latest ChatText child
    /// provides the content. Messages with ChatDeleted as latest child are skipped.
    pub fn messages_from_nodes(
        store: &mut NodeStore,
        chat_path: &str,
        our_key: &str,
        peer_key: &str,
    ) -> HashMap<String, ChatMessage> {
        let mut result = HashMap::new();

        let msg_paths: Vec<String> = store
            .children(chat_path)
            .iter()
            .map(|n| n.path.clone())
            .collect();

        for msg_path in msg_paths {
            let msg_node = match store.get(&msg_path) {
                Some(n) => n.clone(),
                None => continue,
            };

            let msg_id = match msg_path.rsplit('/').next() {
                Some(id) => id.to_string(),
                None => continue,
            };

            let children = store.children(&msg_path);
            let latest = match children.last() {
                Some(n) => n,
                None => continue,
            };

            match &latest.payload {
                HoshiNodePayload::ChatText { content } => {
                    let to = if msg_node.from == our_key {
                        peer_key.to_string()
                    } else {
                        our_key.to_string()
                    };

                    let created_at = uuid::Uuid::parse_str(&msg_id)
                        .ok()
                        .and_then(|u| u.get_timestamp())
                        .map(|ts| ts.to_unix().0 as i64)
                        .unwrap_or(0);

                    result.insert(
                        msg_id.clone(),
                        ChatMessage::new(
                            msg_id,
                            created_at,
                            msg_node.from.clone(),
                            to,
                            content.clone(),
                        ),
                    );
                }
                HoshiNodePayload::ChatDeleted => {
                    // Deleted message — skip
                }
                _ => {}
            }
        }

        result
    }
}
