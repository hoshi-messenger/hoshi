use std::collections::BTreeMap;
use std::path::PathBuf;

use redb::{Database, TableDefinition};
use serde::{Deserialize, Serialize};

const NODES_TABLE: TableDefinition<&str, &[u8]> = TableDefinition::new("nodes");
const HASHES_TABLE: TableDefinition<&str, &[u8; 32]> = TableDefinition::new("hashes");

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct HoshiNode {
    pub from: String,
    #[serde(skip)]
    pub path: String,
    pub payload: HoshiNodePayload,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum HoshiNodePayload {
    Message,
    ChatDeleted,
    ChatText { content: String },
    Config { key: String, value: String },
    Contact,
    ContactPublicKey(String),
    ContactAlias(String),
    ContactDeleted,
    ContactType(String),
}

pub struct NodeStore {
    nodes: BTreeMap<String, HoshiNode>,
    hashes: BTreeMap<String, blake3::Hash>,
    db: Option<Database>,
    public_key: String,
}

impl NodeStore {
    pub fn new(root: Option<PathBuf>, public_key: String) -> Self {
        let db = root.map(|path| {
            std::fs::create_dir_all(&path).expect("failed to create node store directory");
            Database::create(path.join("nodes.redb")).expect("failed to open redb database")
        });
        Self {
            nodes: BTreeMap::new(),
            hashes: BTreeMap::new(),
            db,
            public_key,
        }
    }

    pub fn insert(&mut self, node: HoshiNode) {
        // Collect ancestor paths to invalidate
        let ancestors: Vec<String> = {
            let mut result = Vec::new();
            let mut current = node.path.as_str();
            while let Some(pos) = current.rfind('/') {
                current = &node.path[..pos];
                result.push(current.to_string());
            }
            result
        };

        // Single DB transaction for node write + hash invalidation
        if let Some(db) = &self.db {
            let data = rmp_serde::to_vec(&node).unwrap_or_default();
            let write_txn = db.begin_write().unwrap();
            {
                let mut nodes_table = write_txn.open_table(NODES_TABLE).unwrap();
                nodes_table
                    .insert(node.path.as_str(), data.as_slice())
                    .unwrap();
            }
            {
                let mut hashes_table = write_txn.open_table(HASHES_TABLE).unwrap();
                let _ = hashes_table.remove(node.path.as_str());
                for ancestor in &ancestors {
                    let _ = hashes_table.remove(ancestor.as_str());
                }
            }
            write_txn.commit().unwrap();
        }

        // Update in-memory state
        self.hashes.remove(&node.path);
        for ancestor in &ancestors {
            self.hashes.remove(ancestor);
        }
        self.nodes.insert(node.path.clone(), node);
    }

    pub fn get(&mut self, path: &str) -> Option<&HoshiNode> {
        if !self.nodes.contains_key(path) {
            if let Some(node) = self.read_node_from_disk(path) {
                self.nodes.insert(path.to_string(), node);
            }
        }
        self.nodes.get(path)
    }

    /// Returns all direct children of the given path (one level deep).
    pub fn children(&mut self, path: &str) -> Vec<&HoshiNode> {
        self.load_children_from_disk(path);
        let prefix = format!("{path}/");
        let prefix_len = prefix.len();
        self.nodes
            .range(prefix..)
            .take_while(|(k, _)| {
                k.starts_with(&path) && k.as_bytes().get(path.len()) == Some(&b'/')
            })
            .filter(|(k, _)| !k[prefix_len..].contains('/'))
            .map(|(_, v)| v)
            .collect()
    }

    /// Compute (or return memoized) hash for a path.
    /// Leaf nodes: hash of their serialized payload.
    /// Nodes with children: hash of concatenated child hashes.
    pub fn hash(&mut self, path: &str) -> blake3::Hash {
        if let Some(&h) = self.hashes.get(path) {
            return h;
        }
        if let Some(h) = self.read_hash_from_disk(path) {
            self.hashes.insert(path.to_string(), h);
            return h;
        }

        let child_paths: Vec<String> = {
            self.load_children_from_disk(path);
            self.child_paths_in_memory(path).map(String::from).collect()
        };

        let h = if child_paths.is_empty() {
            let data = self
                .nodes
                .get(path)
                .or_else(|| {
                    // Ensure loaded from disk
                    None
                })
                .map(|n| rmp_serde::to_vec(n).unwrap_or_default())
                .unwrap_or_default();
            blake3::hash(&data)
        } else {
            let mut hasher = blake3::Hasher::new();
            for child_path in child_paths {
                let child_hash = self.hash(&child_path);
                hasher.update(child_hash.as_bytes());
            }
            hasher.finalize()
        };

        self.write_hash_to_disk(path, &h);
        self.hashes.insert(path.to_string(), h);
        h
    }

    /// Set a hash for a path directly, for when we know the hash
    /// but don't have the data locally (e.g. from a remote peer during sync).
    pub fn set_hash(&mut self, path: String, hash: blake3::Hash) {
        self.write_hash_to_disk(&path, &hash);
        self.hashes.insert(path, hash);
    }

    /// Get a previously computed/set hash without recomputing.
    pub fn get_hash(&mut self, path: &str) -> Option<blake3::Hash> {
        if let Some(&h) = self.hashes.get(path) {
            return Some(h);
        }
        if let Some(h) = self.read_hash_from_disk(path) {
            self.hashes.insert(path.to_string(), h);
            return Some(h);
        }
        None
    }

    pub fn set_public_key(&mut self, key: String) {
        self.public_key = key;
    }

    pub fn config_get(&mut self, key: &str) -> Option<String> {
        let path = format!("/config/{key}");
        self.get(&path).and_then(|node| match &node.payload {
            HoshiNodePayload::Config { value, .. } => Some(value.clone()),
            _ => None,
        })
    }

    pub fn config_set(&mut self, key: &str, value: &str) {
        let path = format!("/config/{key}");
        self.insert(HoshiNode {
            from: String::new(),
            path,
            payload: HoshiNodePayload::Config {
                key: key.to_string(),
                value: value.to_string(),
            },
        });
    }

    pub fn contacts(&mut self) -> Vec<crate::Contact> {
        let contact_paths: Vec<String> = self
            .children("/contact")
            .iter()
            .map(|n| n.path.clone())
            .collect();

        let mut contacts = Vec::new();
        for cp in contact_paths {
            let children: Vec<(String, HoshiNodePayload)> = self
                .children(&cp)
                .iter()
                .map(|n| (n.path.clone(), n.payload.clone()))
                .collect();

            // Sort by path (UUIDs are v7, so lexicographic = chronological)
            let mut children = children;
            children.sort_by(|a, b| a.0.cmp(&b.0));

            let mut public_key: Option<String> = None;
            let mut alias: Option<String> = None;
            let mut contact_type: Option<String> = None;
            let mut deleted = false;

            for (_, payload) in &children {
                match payload {
                    HoshiNodePayload::ContactPublicKey(pk) => public_key = Some(pk.clone()),
                    HoshiNodePayload::ContactAlias(a) => alias = Some(a.clone()),
                    HoshiNodePayload::ContactDeleted => deleted = true,
                    HoshiNodePayload::ContactType(t) => contact_type = Some(t.clone()),
                    _ => {}
                }
            }

            if deleted {
                continue;
            }
            if let Some(pk) = public_key {
                let mut contact = crate::Contact::new(pk, alias);
                match contact_type.as_deref() {
                    Some("unknown") => contact.contact_type = crate::ContactType::Unknown,
                    Some("blocked") => contact.contact_type = crate::ContactType::Blocked,
                    _ => {}
                }
                contacts.push(contact);
            }
        }
        contacts
    }

    pub fn contact_upsert(&mut self, contact: &crate::Contact) {
        let contact_path = format!("/contact/{}", contact.public_key);
        let pk_uuid = uuid::Uuid::now_v7().to_string();
        let alias_uuid = uuid::Uuid::now_v7().to_string();
        let type_uuid = uuid::Uuid::now_v7().to_string();

        let type_str = match contact.contact_type {
            crate::ContactType::Unknown => "unknown",
            crate::ContactType::Contact => "contact",
            crate::ContactType::Blocked => "blocked",
        };

        self.insert(HoshiNode {
            from: String::new(),
            path: contact_path.clone(),
            payload: HoshiNodePayload::Contact,
        });
        self.insert(HoshiNode {
            from: String::new(),
            path: format!("{contact_path}/{pk_uuid}"),
            payload: HoshiNodePayload::ContactPublicKey(contact.public_key.clone()),
        });
        self.insert(HoshiNode {
            from: String::new(),
            path: format!("{contact_path}/{alias_uuid}"),
            payload: HoshiNodePayload::ContactAlias(contact.alias.clone()),
        });
        self.insert(HoshiNode {
            from: String::new(),
            path: format!("{contact_path}/{type_uuid}"),
            payload: HoshiNodePayload::ContactType(type_str.to_string()),
        });
    }

    pub fn contact_delete(&mut self, public_key: &str) {
        let contact_path = format!("/contact/{public_key}");
        let del_uuid = uuid::Uuid::now_v7().to_string();
        self.insert(HoshiNode {
            from: String::new(),
            path: format!("{contact_path}/{del_uuid}"),
            payload: HoshiNodePayload::ContactDeleted,
        });
    }

    pub fn may_read(&self, peer_key: &str, path: &str) -> bool {
        if path.starts_with("/config/") || path.starts_with("/contact/") {
            return false;
        }
        self.is_chat_participant(peer_key, path)
    }

    pub fn may_write(&self, peer_key: &str, path: &str, _node: &HoshiNode) -> bool {
        if path.starts_with("/config/") || path.starts_with("/contact/") {
            return false;
        }
        self.is_chat_participant(peer_key, path)
    }

    /// Check if `peer_key` is the other participant in a `/chat/{xor_hash}/...` path.
    /// Returns false for non-chat paths.
    fn is_chat_participant(&self, peer_key: &str, path: &str) -> bool {
        let path = path.strip_prefix("/chat/").unwrap_or_default();
        let xor_hex = match path.split('/').next() {
            Some(s) if !s.is_empty() => s,
            _ => return false,
        };

        let Some(xor_bytes) = hex_decode(xor_hex) else {
            return false;
        };
        let Some(own_bytes) = hex_decode(&self.public_key) else {
            return false;
        };
        if xor_bytes.len() != own_bytes.len() {
            return false;
        }

        let other: Vec<u8> = xor_bytes
            .iter()
            .zip(own_bytes.iter())
            .map(|(a, b)| a ^ b)
            .collect();

        let other_hex: String = other.iter().map(|b| format!("{:02x}", b)).collect();
        other_hex == peer_key
    }

    fn child_paths_in_memory<'a>(&'a self, path: &'a str) -> impl Iterator<Item = &'a str> {
        let prefix = format!("{path}/");
        let prefix_len = prefix.len();
        self.nodes
            .range(prefix..)
            .take_while(move |(k, _)| {
                k.starts_with(&path) && k.as_bytes().get(path.len()) == Some(&b'/')
            })
            .filter(move |(k, _)| !k[prefix_len..].contains('/'))
            .map(|(k, _)| k.as_str())
    }

    // -- redb helpers --

    fn read_node_from_disk(&self, path: &str) -> Option<HoshiNode> {
        let db = self.db.as_ref()?;
        let read_txn = db.begin_read().ok()?;
        let table = read_txn.open_table(NODES_TABLE).ok()?;
        let value = table.get(path).ok()??;
        let mut node: HoshiNode = rmp_serde::from_slice(value.value()).ok()?;
        node.path = path.to_string();
        Some(node)
    }

    fn write_hash_to_disk(&self, path: &str, hash: &blake3::Hash) {
        if let Some(db) = &self.db {
            let write_txn = db.begin_write().unwrap();
            {
                let mut table = write_txn.open_table(HASHES_TABLE).unwrap();
                table.insert(path, hash.as_bytes()).unwrap();
            }
            write_txn.commit().unwrap();
        }
    }

    fn read_hash_from_disk(&self, path: &str) -> Option<blake3::Hash> {
        let db = self.db.as_ref()?;
        let read_txn = db.begin_read().ok()?;
        let table = read_txn.open_table(HASHES_TABLE).ok()?;
        let value = table.get(path).ok()??;
        Some(blake3::Hash::from(*value.value()))
    }

    fn load_children_from_disk(&mut self, path: &str) {
        let db = match &self.db {
            Some(db) => db,
            None => return,
        };
        let read_txn = match db.begin_read() {
            Ok(t) => t,
            Err(_) => return,
        };
        let table = match read_txn.open_table(NODES_TABLE) {
            Ok(t) => t,
            Err(_) => return,
        };

        let prefix = format!("{path}/");
        let prefix_len = prefix.len();
        let range_end = format!("{path}0"); // '0' is the char after '/'

        let mut to_insert = Vec::new();
        if let Ok(range) = table.range::<&str>(prefix.as_str()..range_end.as_str()) {
            for entry in range {
                let Ok(entry) = entry else { continue };
                let key = entry.0.value().to_string();
                if key[prefix_len..].contains('/') {
                    continue;
                }
                if self.nodes.contains_key(&key) {
                    continue;
                }
                let data = entry.1.value().to_vec();
                if let Ok(mut node) = rmp_serde::from_slice::<HoshiNode>(&data) {
                    node.path = key.clone();
                    to_insert.push((key, node));
                }
            }
        }
        for (key, node) in to_insert {
            self.nodes.insert(key, node);
        }
    }
}

fn hex_decode(s: &str) -> Option<Vec<u8>> {
    if s.len() % 2 != 0 {
        return None;
    }
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).ok())
        .collect()
}

fn hex_encode(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{:02x}", b)).collect()
}

/// Compute the chat path for a 1:1 chat between two public keys.
/// Returns `/chat/{a_key XOR b_key}`.
pub fn chat_path(a: &str, b: &str) -> String {
    let a_bytes = hex_decode(a).expect("invalid hex public key");
    let b_bytes = hex_decode(b).expect("invalid hex public key");
    assert_eq!(a_bytes.len(), b_bytes.len(), "public key length mismatch");
    let xor: Vec<u8> = a_bytes
        .iter()
        .zip(b_bytes.iter())
        .map(|(x, y)| x ^ y)
        .collect();
    format!("/chat/{}", hex_encode(&xor))
}

/// Derive the peer's public key from a `/chat/{xor_hex}` path and our own key.
pub fn peer_key_from_chat_path(own_key: &str, path: &str) -> Option<String> {
    let xor_hex = path.strip_prefix("/chat/")?;
    let xor_hex = xor_hex.split('/').next()?;
    let xor_bytes = hex_decode(xor_hex)?;
    let own_bytes = hex_decode(own_key)?;
    if xor_bytes.len() != own_bytes.len() {
        return None;
    }
    let peer: Vec<u8> = xor_bytes
        .iter()
        .zip(own_bytes.iter())
        .map(|(a, b)| a ^ b)
        .collect();
    Some(peer.iter().map(|b| format!("{:02x}", b)).collect())
}
