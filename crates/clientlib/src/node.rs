use std::collections::{BTreeMap, HashSet};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

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
    ChatText { content: String },
    Config { key: String, value: String },
    Contact,
    ContactPublicKey(String),
    ContactType(crate::ContactType),
    Title(String),
}

/// On-disk record format (includes path, unlike HoshiNode which skips it).
#[derive(Serialize, Deserialize)]
struct DiskNode {
    from: String,
    path: String,
    payload: HoshiNodePayload,
}

pub struct HoshiINode {
    node: Option<HoshiNode>,
    hash: Option<blake3::Hash>,
    pub children: BTreeMap<String, HoshiINode>,
}

impl HoshiINode {
    fn new() -> Self {
        Self {
            node: None,
            hash: None,
            children: BTreeMap::new(),
        }
    }

    fn navigate(&self, segments: &[&str]) -> Option<&HoshiINode> {
        let mut current = self;
        for seg in segments {
            current = current.children.get(*seg)?;
        }
        Some(current)
    }

    fn navigate_mut(&mut self, segments: &[&str]) -> &mut HoshiINode {
        let mut current = self;
        for seg in segments {
            current = current
                .children
                .entry(seg.to_string())
                .or_insert_with(HoshiINode::new);
        }
        current
    }
}

enum FileTarget {
    Root,
    Chat(String),
}

fn target_file(path: &str) -> FileTarget {
    if let Some(rest) = path.strip_prefix("/chat/") {
        if let Some(chat_id) = rest.split('/').next() {
            if !chat_id.is_empty() {
                return FileTarget::Chat(chat_id.to_string());
            }
        }
    }
    FileTarget::Root
}

fn parse_path(path: &str) -> Vec<&str> {
    path.split('/').filter(|s| !s.is_empty()).collect()
}

/// Read all records from a `.dat` file, returning HoshiNodes with full paths.
fn load_file(file_path: &std::path::Path, path_prefix: &str) -> Vec<HoshiNode> {
    let mut nodes = Vec::new();
    let data = match fs::read(file_path) {
        Ok(d) => d,
        Err(_) => return nodes,
    };
    let mut cursor = 0;
    while cursor + 4 <= data.len() {
        let len = u32::from_le_bytes([
            data[cursor],
            data[cursor + 1],
            data[cursor + 2],
            data[cursor + 3],
        ]) as usize;
        cursor += 4;
        if cursor + len > data.len() {
            break;
        }
        if let Ok(disk_node) = rmp_serde::from_slice::<DiskNode>(&data[cursor..cursor + len]) {
            let full_path = if path_prefix.is_empty() {
                disk_node.path
            } else {
                format!("{path_prefix}{}", disk_node.path)
            };
            nodes.push(HoshiNode {
                from: disk_node.from,
                path: full_path,
                payload: disk_node.payload,
            });
        }
        cursor += len;
    }
    nodes
}

/// Append a single record to a `.dat` file.
fn append_node(file_path: &std::path::Path, disk_node: &DiskNode) {
    let data = rmp_serde::to_vec(disk_node).expect("failed to serialize node");
    let len = (data.len() as u32).to_le_bytes();
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(file_path)
        .expect("failed to open dat file");
    file.write_all(&len).expect("failed to write length");
    file.write_all(&data).expect("failed to write data");
}

pub struct NodeStore {
    root: HoshiINode,
    dir: Option<PathBuf>,
    loaded_chats: HashSet<String>,
    public_key: String,
}

impl NodeStore {
    pub fn new(root: Option<PathBuf>, public_key: String) -> Self {
        let dir = root.map(|path| {
            fs::create_dir_all(&path).expect("failed to create node store directory");
            path
        });

        let mut tree = HoshiINode::new();

        // Load root.dat at startup
        if let Some(dir) = &dir {
            let root_dat = dir.join("root.dat");
            for node in load_file(&root_dat, "") {
                let segments = parse_path(&node.path);
                let inode = tree.navigate_mut(&segments);
                inode.node = Some(node);
            }
        }

        Self {
            root: tree,
            dir,
            loaded_chats: HashSet::new(),
            public_key,
        }
    }

    pub fn insert(&mut self, node: HoshiNode) {
        let segments = parse_path(&node.path);

        // Invalidate hashes up the ancestor chain
        self.root.hash = None;
        {
            let mut current = &mut self.root;
            for seg in &segments {
                current = current
                    .children
                    .entry(seg.to_string())
                    .or_insert_with(HoshiINode::new);
                current.hash = None;
            }
        }

        // Ensure chat file is loaded before appending
        if let FileTarget::Chat(ref chat_id) = target_file(&node.path) {
            self.ensure_chat_loaded(chat_id);
        }

        // Append to disk
        if let Some(dir) = &self.dir {
            let (file_path, stored_path) = match target_file(&node.path) {
                FileTarget::Root => (dir.join("root.dat"), node.path.clone()),
                FileTarget::Chat(ref chat_id) => {
                    let prefix = format!("/chat/{chat_id}/");
                    let stripped = node.path.strip_prefix(&prefix).unwrap_or(&node.path);
                    (
                        dir.join(format!("chat.{chat_id}.dat")),
                        stripped.to_string(),
                    )
                }
            };
            let disk_node = DiskNode {
                from: node.from.clone(),
                path: stored_path,
                payload: node.payload.clone(),
            };
            append_node(&file_path, &disk_node);
        }

        // Insert into tree
        let segments = parse_path(&node.path);
        let inode = self.root.navigate_mut(&segments);
        inode.node = Some(node);
    }

    pub fn get(&mut self, path: &str) -> Option<&HoshiNode> {
        self.ensure_path_loaded(path);
        let segments = parse_path(path);
        self.root.navigate(&segments)?.node.as_ref()
    }

    /// Returns all direct children of the given path (one level deep).
    pub fn children(&mut self, path: &str) -> Vec<&HoshiNode> {
        self.ensure_path_loaded(path);
        let segments = parse_path(path);
        let inode = match self.root.navigate(&segments) {
            Some(n) => n,
            None => return vec![],
        };
        inode
            .children
            .values()
            .filter_map(|child| child.node.as_ref())
            .collect()
    }

    /// Compute (or return memoized) hash for a path.
    /// Leaf nodes: hash of their serialized payload.
    /// Nodes with children: hash of concatenated child hashes.
    pub fn hash(&mut self, path: &str) -> blake3::Hash {
        self.ensure_path_loaded(path);

        // Check cached
        if let Some(h) = self.root.navigate(&parse_path(path)).and_then(|n| n.hash) {
            return h;
        }

        let child_keys: Vec<String> = {
            let segments = parse_path(path);
            match self.root.navigate(&segments) {
                Some(inode) => inode.children.keys().cloned().collect(),
                None => vec![],
            }
        };

        let h = if child_keys.is_empty() {
            let data = self
                .root
                .navigate(&parse_path(path))
                .and_then(|n| n.node.as_ref())
                .map(|n| rmp_serde::to_vec(n).unwrap_or_default())
                .unwrap_or_default();
            blake3::hash(&data)
        } else {
            let mut hasher = blake3::Hasher::new();
            for key in child_keys {
                let child_path = format!("{path}/{key}");
                let child_hash = self.hash(&child_path);
                hasher.update(child_hash.as_bytes());
            }
            hasher.finalize()
        };

        // Cache
        let segments = parse_path(path);
        let inode = self.root.navigate_mut(&segments);
        inode.hash = Some(h);
        h
    }

    /// Set a hash for a path directly, for when we know the hash
    /// but don't have the data locally (e.g. from a remote peer during sync).
    pub fn set_hash(&mut self, path: String, hash: blake3::Hash) {
        let segments = parse_path(&path);
        let inode = self.root.navigate_mut(&segments);
        inode.hash = Some(hash);
    }

    /// Get a previously computed/set hash without recomputing.
    pub fn get_hash(&mut self, path: &str) -> Option<blake3::Hash> {
        let segments = parse_path(path);
        self.root.navigate(&segments)?.hash
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
            let mut contact_type = crate::ContactType::Contact;

            for (_, payload) in &children {
                match payload {
                    HoshiNodePayload::ContactPublicKey(pk) => public_key = Some(pk.clone()),
                    HoshiNodePayload::ContactType(t) => contact_type = t.clone(),
                    _ => {}
                }
            }

            if contact_type == crate::ContactType::Deleted {
                continue;
            }
            if let Some(pk) = public_key {
                let mut contact = crate::Contact::new(pk);
                contact.contact_type = contact_type;
                contacts.push(contact);
            }
        }
        contacts
    }

    pub fn contact_upsert(&mut self, contact: &crate::Contact) {
        let contact_path = format!("/contact/{}", contact.public_key);
        let pk_uuid = uuid::Uuid::now_v7().to_string();
        let type_uuid = uuid::Uuid::now_v7().to_string();

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
            path: format!("{contact_path}/{type_uuid}"),
            payload: HoshiNodePayload::ContactType(contact.contact_type.clone()),
        });
    }

    pub fn user_alias_set(&mut self, alias: &str) -> bool {
        if self.user_alias_get(&self.public_key.clone()) == Some(alias.to_string()) {
            return false;
        }
        let path = user_path(&self.public_key);
        let uuid = uuid::Uuid::now_v7().to_string();
        self.insert(HoshiNode {
            from: self.public_key.clone(),
            path: format!("{path}/{uuid}"),
            payload: HoshiNodePayload::Title(alias.to_string()),
        });
        true
    }

    pub fn user_alias_get(&mut self, public_key: &str) -> Option<String> {
        let path = user_path(public_key);
        let children: Vec<(String, HoshiNodePayload)> = self
            .children(&path)
            .iter()
            .map(|n| (n.path.clone(), n.payload.clone()))
            .collect();
        let mut children = children;
        children.sort_by(|a, b| a.0.cmp(&b.0));
        let mut alias = None;
        for (_, payload) in &children {
            if let HoshiNodePayload::Title(a) = payload {
                alias = Some(a.clone());
            }
        }
        alias
    }

    pub fn contact_delete(&mut self, public_key: &str) {
        let contact_path = format!("/contact/{public_key}");
        let del_uuid = uuid::Uuid::now_v7().to_string();
        self.insert(HoshiNode {
            from: String::new(),
            path: format!("{contact_path}/{del_uuid}"),
            payload: HoshiNodePayload::ContactType(crate::ContactType::Deleted),
        });
    }

    pub fn may_read(&self, peer_key: &str, path: &str) -> bool {
        if path.starts_with("/config/") || path.starts_with("/contact/") {
            return false;
        }
        if path.starts_with("/user/") {
            return true;
        }
        self.is_chat_participant(peer_key, path)
    }

    pub fn may_write(&self, peer_key: &str, path: &str, _node: &HoshiNode) -> bool {
        if path.starts_with("/config/") || path.starts_with("/contact/") {
            return false;
        }
        if path.starts_with("/user/") {
            return self.is_user_owner(peer_key, path);
        }
        self.is_chat_participant(peer_key, path)
    }

    fn is_user_owner(&self, peer_key: &str, path: &str) -> bool {
        let rest = path.strip_prefix("/user/").unwrap_or_default();
        let pk = rest.split('/').next().unwrap_or_default();
        peer_key == pk
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

    fn ensure_path_loaded(&mut self, path: &str) {
        if let FileTarget::Chat(chat_id) = target_file(path) {
            self.ensure_chat_loaded(&chat_id);
        }
    }

    fn ensure_chat_loaded(&mut self, chat_id: &str) {
        if self.loaded_chats.contains(chat_id) {
            return;
        }
        self.loaded_chats.insert(chat_id.to_string());

        if let Some(dir) = &self.dir {
            let file_path = dir.join(format!("chat.{chat_id}.dat"));
            let prefix = format!("/chat/{chat_id}/");
            let nodes = load_file(&file_path, &prefix);
            for node in nodes {
                let segments = parse_path(&node.path);
                let inode = self.root.navigate_mut(&segments);
                inode.node = Some(node);
            }
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

pub fn user_path(public_key: &str) -> String {
    format!("/user/{public_key}")
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
