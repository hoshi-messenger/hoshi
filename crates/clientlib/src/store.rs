use std::{
    collections::{BTreeMap, HashMap},
    fs::File,
    io::{BufWriter, Write},
    path::{Path, PathBuf},
};

use anyhow::Result;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

pub trait Store: serde::Serialize + serde::de::DeserializeOwned + std::fmt::Debug {
    fn msg_id(&self) -> Uuid;
    fn msg_hash(&self) -> blake3::Hash;
}

pub struct StoreHead<T: Store> {
    name: String,
    file: Option<BufWriter<File>>,
    messages: BTreeMap<Uuid, T>,
    hashes: HashMap<Uuid, blake3::Hash>,
    remotes: HashMap<String, StoreHeadRemote>,
}

pub struct StoreHeadRemote {
    pub key: String,
    pub tip: blake3::Hash,
}

impl StoreHeadRemote {
    pub fn new(key: String, tip: blake3::Hash) -> Self {
        Self { key, tip }
    }
}

#[derive(Deserialize, Serialize, Clone, Debug)]
#[serde(bound = "T: Store")]
pub enum HeadCommand<T: Store> {
    Put(Vec<T>),
    Tip([u8; 32]),
}

impl<T: Store> StoreHead<T> {
    pub fn new(name: String, repo_path: Option<&Path>) -> Self {
        let file = repo_path.clone().map(|p| {
            let p = p.join(format!("{name}.hoshi"));
            BufWriter::new(
                File::options()
                    .append(true)
                    .create(true)
                    .open(p)
                    .expect("Couldn't open StoreHead file"),
            )
        });
        let messages = repo_path
            .map(|p| {
                let p = p.join(format!("{name}.hoshi"));
                Self::load_messages(p).expect("Couldn't load local StoreHead")
            })
            .unwrap_or_default();

        Self {
            name,
            file,
            messages,
            hashes: HashMap::new(),
            remotes: HashMap::new(),
        }
    }

    pub fn add_remote(&mut self, key: String, tip: Option<blake3::Hash>) {
        let tip = tip.unwrap_or_else(|| self.hash_start());
        self.remotes
            .entry(key.clone())
            .or_insert_with(move || StoreHeadRemote::new(key, tip));
    }

    pub fn get(&self, uuid: Uuid) -> Option<&T> {
        self.messages.get(&uuid)
    }

    pub fn len(&self) -> usize {
        self.messages.len()
    }

    fn hash_start(&self) -> blake3::Hash {
        blake3::Hasher::new()
            .update(self.name.as_bytes())
            .finalize()
    }

    fn load_messages(path: PathBuf) -> Result<BTreeMap<Uuid, T>> {
        let mut map = BTreeMap::new();
        let data = std::fs::read(path)?;
        let mut i = 0;
        while i + 4 < data.len() {
            let len = &data[i..i + 4].try_into()?;
            let len = u32::from_le_bytes(*len) as usize;
            i = i + 4;
            let msg = &data[i..i + len];
            let msg = rmp_serde::from_slice::<T>(msg)?;
            map.insert(msg.msg_id(), msg);
            i = i + len;
        }
        Ok(map)
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
        };
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
        let cur = blake3::Hasher::new()
            .update(prev.as_bytes())
            .update(cur.as_bytes())
            .finalize();
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
        if let Some(file) = self.file.as_mut() {
            match rmp_serde::encode::to_vec(&msg) {
                Ok(encoded) => {
                    let len = (encoded.len() as u32).to_le_bytes();
                    file.write_all(&len).expect("Couldn't write len");
                    file.write_all(encoded.as_slice())
                        .expect("Couldn't write msgpack");
                }
                Err(err) => {
                    eprintln!(
                        "Error enconding msg for store {}: {:?} - Msg: {:?}",
                        self.name, err, msg,
                    );
                }
            }
        }
        self.messages.insert(uuid, msg);
        // invalidate cached hashes for uuid and everything after
        self.hashes.retain(|id, _| id < &uuid);
    }

    pub fn rx(&mut self, from: String, cmd: HeadCommand<T>) {
        if !self.remotes.contains_key(&from) {
            self.add_remote(from.clone(), None);
        };
        let Some(remote) = self.remotes.get(&from) else {
            panic!("add_remote didn't work!");
        };
    }

    pub fn tx(&mut self, tx: impl FnMut(String, HeadCommand<T>)) {
        for remote in self.remotes.values_mut() {}
    }
}
