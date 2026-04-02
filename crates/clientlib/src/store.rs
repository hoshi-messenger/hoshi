use std::{
    collections::{BTreeMap, HashMap},
    fs::File,
    io::BufWriter,
    path::PathBuf,
};

use serde::{Deserialize, Serialize};
use uuid::Uuid;

pub trait Store: serde::Serialize + std::fmt::Debug {
    fn msg_id(&self) -> Uuid;
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

#[derive(Deserialize, Serialize, Clone, Debug)]
pub struct RepoCommand<T: Store> {
    pub head: String,
    pub commands: Vec<HeadCommand<T>>,
}

impl<T: Store> StoreHead<T> {
    pub fn new(name: String, repo_path: Option<PathBuf>) -> Self {
        let file = repo_path.map(|p| {
            let p = p.join(format!("{name}.hoshi"));
            BufWriter::new(
                File::options()
                    .append(true)
                    .create(true)
                    .open(p)
                    .expect("Couldn't open StoreHead file"),
            )
        });

        Self {
            name,
            file,
            hashes: HashMap::new(),
            messages: BTreeMap::new(),
        }
    }

    pub fn get(&self, uuid: Uuid) -> Option<&T> {
        self.messages.get(&uuid)
    }

    pub fn len(&self) -> usize {
        self.messages.len()
    }

    fn hash_start(&self) -> blake3::Hash {
        let mut hasher = blake3::Hasher::new();
        hasher.update(self.name.as_bytes());
        hasher.finalize()
    }

    pub fn hash_tip(&mut self) -> blake3::Hash {
        if let Some(last) = self.messages.last_entry() {
            let uuid = *last.key();
            self.hash(uuid).expect("Last entry couldn't be hashed")
        } else {
            self.hash_start()
        }
    }

    pub fn hash(&mut self, uuid: Uuid) -> Option<blake3::Hash> {
        if let Some(hash) = self.hashes.get(&uuid) {
            return Some(*hash);
        }
        let mut range = self.messages.range(uuid..);
        let Some(cur) = range.next() else {
            return None;
        };
        let cur = cur.1.msg_hash();

        let prev = match range.next_back() {
            None => self.hash_start(),
            Some((id, _)) => {
                let id = *id;
                self.hash(id)
                    .expect("Couldn't hash entry that was in StoreHead")
            }
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
