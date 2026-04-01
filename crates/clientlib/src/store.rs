use std::{
    collections::{BTreeMap, HashMap},
    fs::File,
    io::BufWriter,
    ops::Bound::{Excluded, Unbounded},
    path::PathBuf,
};

use serde::{Deserialize, Serialize};
use uuid::Uuid;

pub trait Store: serde::Serialize {
    fn msg_id(&self) -> Uuid;
    fn msg_from(&self) -> String;
    fn msg_hash(&self) -> blake3::Hash;
}

pub struct StoreHead<T: Store> {
    name: String,
    file: Option<BufWriter<File>>,
    messages: BTreeMap<Uuid, T>,
    hashes: HashMap<Uuid, blake3::Hash>,
}

#[derive(Deserialize, Serialize, Clone, Debug)]
pub enum HeadCommand<T: Store> {
    Has(Vec<(Uuid, [u8; 32])>),
    Get(Vec<Uuid>),
    Put(Vec<T>),
}

pub struct RepoCommand<T: Store> {
    pub head: String,
    pub commands: Vec<HeadCommand<T>>,
}

impl<T: Store> StoreHead<T> {
    pub fn new(name: String, repo_path: Option<PathBuf>) -> Self {
        let file = None;
        Self {
            name,
            file,
            hashes: HashMap::new(),
            messages: BTreeMap::new(),
        }
    }

    pub fn hash(&mut self, uuid: Uuid) -> Option<blake3::Hash> {
        if let Some(hash) = self.hashes.get(&uuid) {
            return Some(*hash);
        }
        let mut range = self.messages.range(..uuid);
        let Some(cur) = range.next_back() else {
            return None;
        };
        let cur = cur.1.msg_hash();
        let Some(prev) = range.next_back() else {
            self.hashes.insert(uuid, cur);
            return Some(cur);
        };
        let Some(prev) = self.hash(*prev.0) else {
            panic!(
                "Could get msg but couldn't hash it, this should never happen: {}:{}",
                self.name, uuid,
            )
        };
        let mut hasher = blake3::Hasher::new();
        hasher.update(prev.as_bytes());
        hasher.update(cur.as_bytes());
        let cur = hasher.finalize();
        self.hashes.insert(uuid, cur);
        Some(cur)
    }

    pub fn insert(&mut self, msg: T) {
        let uuid = msg.msg_id();
        if self.messages.contains_key(&uuid) {
            if cfg!(debug_assertions) {
                eprintln!("Duplicate insert: {}:{}", self.name, uuid);
            }
            return;
        }
        if let Some(mut file) = self.file.as_mut() {
            if let Err(err) = rmp_serde::encode::write(&mut file, &msg) {
                eprintln!("Error writing to store {}: {:?}", self.name, err);
            }
        }
        self.messages.insert(uuid, msg);
        // invalidate cached hashes for uuid and everything after
        self.hashes.retain(|id, _| id < &uuid);

        let mut hash = self
            .hash(uuid)
            .expect("Just inserted msg should have a hash");
        for (id, msg) in self.messages.range((Excluded(uuid), Unbounded)) {
            let mut hasher = blake3::Hasher::new();
            hasher.update(hash.as_bytes());
            hasher.update(msg.msg_hash().as_bytes());
            hash = hasher.finalize();
            self.hashes.insert(*id, hash);
        }
    }

    pub fn exec(&mut self, cmd: HeadCommand<T>) {
        match cmd {
            HeadCommand::Put(v) => v.into_iter().for_each(|msg| self.insert(msg)),
            _ => unimplemented!("HeadCommand"),
        }
    }
}

pub struct StoreRepo<T: Store> {
    persistence_path: Option<PathBuf>,
    stores: HashMap<String, StoreHead<T>>,
}

impl<T: Store> StoreRepo<T> {
    pub fn new(persistence_path: Option<PathBuf>) -> Self {
        Self {
            persistence_path,
            stores: HashMap::new(),
        }
    }
}
