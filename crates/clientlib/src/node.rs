use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct HoshiNode {
    pub from: String,
    pub path: String,
    pub payload: HoshiNodePayload,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum HoshiNodePayload {
    Message,
    ChatDeleted,
    ChatText { content: String },
}

pub struct NodeStore {
    nodes: BTreeMap<String, HoshiNode>,
    hashes: BTreeMap<String, blake3::Hash>,
    root: Option<PathBuf>,
    public_key: String,
}

impl NodeStore {
    pub fn new(root: Option<PathBuf>, public_key: String) -> Self {
        if let Some(root) = &root {
            std::fs::create_dir_all(root).expect("failed to create node store root directory");
        }
        Self {
            nodes: BTreeMap::new(),
            hashes: BTreeMap::new(),
            root,
            public_key,
        }
    }

    pub fn insert(&mut self, node: HoshiNode) {
        self.invalidate_ancestors(&node.path);
        self.hashes.remove(&node.path);
        self.write_node_to_disk(&node);
        self.remove_hash_from_disk(&node.path);
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
            self.child_paths_in_memory(path)
                .map(String::from)
                .collect()
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

    pub fn may_read(&self, peer_key: &str, path: &str) -> bool {
        self.is_chat_participant(peer_key, path)
    }

    pub fn may_write(&self, peer_key: &str, path: &str, _node: &HoshiNode) -> bool {
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

    fn invalidate_ancestors(&mut self, path: &str) {
        let mut current = path;
        while let Some(pos) = current.rfind('/') {
            current = &path[..pos];
            self.hashes.remove(current);
            self.remove_hash_from_disk(current);
        }
    }

    // -- Filesystem helpers --

    fn dir_for_path(&self, path: &str) -> Option<PathBuf> {
        self.root.as_ref().map(|root| {
            let relative = path.strip_prefix('/').unwrap_or(path);
            if relative.is_empty() {
                root.clone()
            } else {
                root.join(relative)
            }
        })
    }

    fn write_node_to_disk(&self, node: &HoshiNode) {
        if let Some(dir) = self.dir_for_path(&node.path) {
            let _ = std::fs::create_dir_all(&dir);
            let data = rmp_serde::to_vec(node).unwrap_or_default();
            let _ = std::fs::write(dir.join("__CONTENT__"), &data);
        }
    }

    fn read_node_from_disk(&self, path: &str) -> Option<HoshiNode> {
        let dir = self.dir_for_path(path)?;
        let data = std::fs::read(dir.join("__CONTENT__")).ok()?;
        rmp_serde::from_slice(&data).ok()
    }

    fn write_hash_to_disk(&self, path: &str, hash: &blake3::Hash) {
        if let Some(dir) = self.dir_for_path(path) {
            let _ = std::fs::create_dir_all(&dir);
            let _ = std::fs::write(dir.join("__HASH__"), hash.as_bytes());
        }
    }

    fn read_hash_from_disk(&self, path: &str) -> Option<blake3::Hash> {
        let dir = self.dir_for_path(path)?;
        let data = std::fs::read(dir.join("__HASH__")).ok()?;
        let bytes: [u8; 32] = data.try_into().ok()?;
        Some(blake3::Hash::from(bytes))
    }

    fn remove_hash_from_disk(&self, path: &str) {
        if let Some(dir) = self.dir_for_path(path) {
            let _ = std::fs::remove_file(dir.join("__HASH__"));
        }
    }

    fn load_children_from_disk(&mut self, path: &str) {
        let dir = match self.dir_for_path(path) {
            Some(d) => d,
            None => return,
        };
        let entries = match std::fs::read_dir(&dir) {
            Ok(e) => e,
            Err(_) => return,
        };
        for entry in entries.flatten() {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if name == "__CONTENT__" || name == "__HASH__" {
                continue;
            }
            if !entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                continue;
            }
            let child_path = format!("{path}/{name}");
            if self.nodes.contains_key(&child_path) {
                continue;
            }
            let content_file = dir.join(name.as_ref()).join("__CONTENT__");
            if let Ok(data) = std::fs::read(&content_file) {
                if let Ok(node) = rmp_serde::from_slice::<HoshiNode>(&data) {
                    self.nodes.insert(child_path, node);
                }
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

/// Compute the chat path for a 1:1 chat between two public keys.
/// Returns `/chat/{a_key XOR b_key}`.
pub fn chat_path(a: &str, b: &str) -> String {
    let a_bytes = hex_decode(a).expect("invalid hex public key");
    let b_bytes = hex_decode(b).expect("invalid hex public key");
    assert_eq!(a_bytes.len(), b_bytes.len(), "public key length mismatch");
    let xor: Vec<u8> = a_bytes.iter().zip(b_bytes.iter()).map(|(x, y)| x ^ y).collect();
    format!("/chat/{}", hex_encode(&xor))
}
