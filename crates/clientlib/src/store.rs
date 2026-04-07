use std::{
    collections::{BTreeMap, HashMap},
    fs::File,
    io::{BufWriter, Write},
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

use anyhow::Result;
use bimap::BiHashMap;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

pub trait Store: serde::Serialize + serde::de::DeserializeOwned + std::fmt::Debug + Clone {
    fn msg_id(&self) -> Uuid;
    fn msg_hash(&self) -> blake3::Hash;
}

pub struct StoreHead<T: Store> {
    name: String,
    file: Option<BufWriter<File>>,
    messages: BTreeMap<Uuid, T>,
    hashes: BiHashMap<Uuid, blake3::Hash>,
    remotes: HashMap<String, StoreHeadRemote>,
}

pub struct StoreHeadRemote {
    pub key: String,
    pub tip: blake3::Hash,
    pub cooldown_until: Instant,
    pub tip_update_queued: bool,
}

impl StoreHeadRemote {
    pub fn new(key: String, tip: blake3::Hash) -> Self {
        Self {
            key,
            tip,
            cooldown_until: Instant::now(),
            tip_update_queued: false,
        }
    }
}

#[derive(Deserialize, Serialize, Clone, Debug)]
#[serde(bound = "T: Store")]
pub enum HeadCommand<T: Store> {
    Put(T),
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
            hashes: BiHashMap::new(),
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
        if let Some(hash) = self.hashes.get_by_left(&uuid) {
            return Some(*hash);
        };
        let Some(cur) = self.messages.get(&uuid) else {
            return None;
        };
        let cur = cur.msg_hash();

        let mut range = self.messages.range_mut(..uuid);
        let prev = match range.next_back() {
            None => {
                if cfg!(debug_assertions) {
                    if let Some(first) = self.messages.first_key_value() {
                        if uuid != *first.0 {
                            panic!(
                                "{} - prev is none, but first entry has uuid {}",
                                uuid, first.0
                            );
                        }
                    }
                }
                self.hash_start()
            }
            Some((id, _)) => {
                let id = *id;
                debug_assert_ne!(id, uuid);
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

    pub fn insert(&mut self, msg: T) -> bool {
        let uuid = msg.msg_id();
        if self.messages.contains_key(&uuid) {
            if cfg!(debug_assertions) {
                eprintln!("Duplicate insert: {}:{}", self.name, uuid);
            }
            return false;
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

        // Reset remote cooldown so we immediately try syncing
        let now = Instant::now();
        for remote in self.remotes.values_mut() {
            remote.cooldown_until = now.clone();
        }
        true
    }

    pub fn rx(&mut self, from: &str, cmd: HeadCommand<T>) {
        if !self.remotes.contains_key(from) {
            self.add_remote(from.to_string(), None);
        };
        let Some(remote) = self.remotes.get_mut(from) else {
            panic!("add_remote didn't work!");
        };

        remote.cooldown_until = Instant::now();
        match cmd {
            HeadCommand::Tip(tip) => {
                remote.tip = blake3::Hash::from_bytes(tip);
            }
            HeadCommand::Put(msg) => {
                if self.insert(msg) {
                    self.remotes
                        .get_mut(from)
                        .map(|r| r.tip_update_queued = true);
                }
            }
        }
    }

    pub fn tx(&mut self, mut tx: impl FnMut(String, HeadCommand<T>)) {
        let tip = self.hash_tip();
        let start = self.hash_start();
        for remote in self.remotes.values_mut() {
            if remote.tip_update_queued {
                remote.tip_update_queued = false;
                tx(remote.key.clone(), HeadCommand::Tip(tip.as_bytes().clone()));
            }

            if remote.tip == tip {
                continue;
            };
            let now = Instant::now();
            if now < remote.cooldown_until {
                continue;
            }
            if remote.tip == start {
                for (_, msg) in self.messages.iter().take(8) {
                    tx(remote.key.clone(), HeadCommand::Put(msg.clone()));
                }
            } else if let Some(uuid) = self.hashes.get_by_right(&remote.tip) {
                for (_, msg) in self.messages.range(uuid..).skip(1).take(8) {
                    tx(remote.key.clone(), HeadCommand::Put(msg.clone()));
                }
            } else {
                for msg in self.messages.values() {
                    eprintln!("{} - {}", msg.msg_id(), msg.msg_hash());
                }
                for hash in self.hashes.right_values() {
                    eprintln!("{}", hash);
                }
                eprintln!("We require '{}' to PUT", &remote.key);
            }
            tx(remote.key.clone(), HeadCommand::Tip(tip.as_bytes().clone()));
            remote.cooldown_until = now + Duration::from_secs(60);
        }
    }
}
