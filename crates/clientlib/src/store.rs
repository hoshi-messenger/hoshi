use std::{
    cell::RefCell,
    collections::{BTreeMap, HashMap},
    fs::File,
    io::{BufWriter, Write},
    path::{Path, PathBuf},
    rc::Rc,
    time::{Duration, Instant},
};

use anyhow::Result;
use bimap::BiHashMap;
use rand::prelude::*;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

pub trait Store: serde::Serialize + serde::de::DeserializeOwned + std::fmt::Debug + Clone {
    fn id(&self) -> Uuid;
    fn hash(&self) -> blake3::Hash;
}

pub struct StoreWatcher<T: Store> {
    pub id: Uuid,
    last_hash: Option<blake3::Hash>,
    fun: Box<dyn Fn(&BTreeMap<Uuid, T>)>,
}

pub struct StoreWatcherRef<T: Store> {
    id: Uuid,
    watchers: Rc<RefCell<Vec<StoreWatcher<T>>>>,
}

impl<T: Store> Drop for StoreWatcherRef<T> {
    fn drop(&mut self) {
        self.watchers.borrow_mut().retain(|w| w.id != self.id);
    }
}

pub struct StoreHead<T: Store> {
    name: String,
    file: Option<BufWriter<File>>,
    messages: BTreeMap<Uuid, T>,
    pending: RefCell<Vec<T>>,
    hashes: BiHashMap<Uuid, blake3::Hash>,
    remotes: HashMap<String, StoreHeadRemote>,
    watchers: Rc<RefCell<Vec<StoreWatcher<T>>>>,
}

pub struct StoreHeadRemote {
    pub key: String,
    pub tip: blake3::Hash,
    pub tip_secondary: blake3::Hash,
    pub cooldown_until: Instant,
    pub tip_update_queued: bool,
}

impl StoreHeadRemote {
    pub fn new(key: String, tip: blake3::Hash) -> Self {
        Self {
            key,
            tip,
            tip_secondary: tip,
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
    TipSecondary([u8; 32]),
}

impl<T: Store> StoreWatcher<T> {
    pub fn new(fun: Box<dyn Fn(&BTreeMap<Uuid, T>)>) -> Self {
        let id = Uuid::now_v7();
        Self {
            id,
            last_hash: None,
            fun,
        }
    }

    pub fn run(&mut self, tip: blake3::Hash, messages: &BTreeMap<Uuid, T>) {
        if let Some(hash) = self.last_hash
            && hash == tip
        {
            return;
        }
        self.last_hash = Some(tip);
        (self.fun)(messages);
    }
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
            pending: RefCell::new(vec![]),
            hashes: BiHashMap::new(),
            remotes: HashMap::new(),
            watchers: Rc::new(RefCell::new(vec![])),
        }
    }

    pub fn watcher_add(
        &mut self,
        fun: impl Fn(&BTreeMap<Uuid, T>) + 'static,
    ) -> StoreWatcherRef<T> {
        let watcher = StoreWatcher::new(Box::new(fun));
        let id = watcher.id;
        self.watchers.borrow_mut().push(watcher);
        self.watcher_run();

        StoreWatcherRef {
            id,
            watchers: self.watchers.clone(),
        }
    }

    pub fn watcher_run(&mut self) {
        let tip = self.hash_tip();
        let messages = &self.messages;
        for watcher in self.watchers.borrow_mut().iter_mut() {
            watcher.run(tip, messages);
        }
    }

    pub fn watcher_len(&self) -> usize {
        self.watchers.borrow().len()
    }

    pub fn remote_add(&mut self, key: String, tip: Option<blake3::Hash>) {
        let tip = tip.unwrap_or_else(|| self.hash_start());
        self.remotes
            .entry(key.clone())
            .or_insert_with(move || StoreHeadRemote::new(key, tip));
    }

    pub fn remote_drop(&mut self, key: &str) {
        self.remotes.remove(key);
    }

    pub fn get(&self, uuid: Uuid) -> Option<&T> {
        self.messages.get(&uuid)
    }

    pub fn get_all(&self) -> &BTreeMap<Uuid, T> {
        &self.messages
    }

    pub fn len(&self) -> usize {
        self.messages.len()
    }

    pub fn queue(&self, msg: T) {
        self.pending.borrow_mut().push(msg);
    }

    fn hash_start(&self) -> blake3::Hash {
        blake3::Hasher::new()
            .update(self.name.as_bytes())
            .finalize()
    }

    fn load_messages(path: PathBuf) -> Result<BTreeMap<Uuid, T>> {
        let mut map = BTreeMap::new();
        let data = std::fs::read(&path)?;
        let mut i = 0;
        while i + 4 <= data.len() {
            let len = &data[i..i + 4].try_into()?;
            let len = u32::from_le_bytes(*len) as usize;
            i = i + 4;
            if i + len > data.len() {
                if cfg!(debug_assertions) {
                    eprintln!(
                        "Ignoring torn final record in StoreHead file {} at byte {}",
                        path.display(),
                        i - 4,
                    );
                }
                break;
            }
            let msg = &data[i..i + len];
            let msg = rmp_serde::from_slice::<T>(msg)?;
            map.insert(msg.id(), msg);
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
        let cur = cur.hash();

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

    fn write_message(&mut self, msg: &T) {
        if let Some(file) = self.file.as_mut() {
            match rmp_serde::encode::to_vec(msg) {
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
    }

    fn flush_file(&mut self) {
        if let Some(file) = self.file.as_mut() {
            file.flush().expect("Couldn't flush StoreHead file");
        }
    }

    fn insert_apply(&mut self, msg: T) -> bool {
        let uuid = msg.id();
        if self.messages.contains_key(&uuid) {
            if cfg!(debug_assertions) {
                eprintln!("Duplicate insert: {}:{}", self.name, uuid);
            }
            return false;
        }
        self.write_message(&msg);
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

    pub fn insert(&mut self, msg: T) -> bool {
        if !self.insert_apply(msg) {
            return false;
        }
        self.flush_file();
        self.watcher_run();
        true
    }

    pub fn step(&mut self) -> usize {
        let pending = std::mem::take(self.pending.get_mut());
        let mut inserted = 0;
        for msg in pending {
            if self.insert_apply(msg) {
                inserted += 1;
            }
        }
        if inserted > 0 {
            self.flush_file();
            self.watcher_run();
        }
        inserted
    }

    pub fn rx(&mut self, from: &str, cmd: HeadCommand<T>) {
        if !self.remotes.contains_key(from) {
            self.remote_add(from.to_string(), None);
        };
        let Some(remote) = self.remotes.get_mut(from) else {
            panic!("add_remote didn't work!");
        };

        remote.cooldown_until = Instant::now();
        let mut changed = false;
        match cmd {
            HeadCommand::Tip(tip) => {
                remote.tip = blake3::Hash::from_bytes(tip);
            }
            HeadCommand::TipSecondary(tip) => {
                remote.tip_secondary = blake3::Hash::from_bytes(tip);
            }
            HeadCommand::Put(msg) => {
                if self.insert_apply(msg) {
                    self.remotes
                        .get_mut(from)
                        .map(|r| r.tip_update_queued = true);
                    self.flush_file();
                    changed = true;
                }
            }
        };
        if changed {
            self.watcher_run();
        }
    }

    pub fn tx(&mut self, mut tx: impl FnMut(String, HeadCommand<T>)) {
        let tip = self.hash_tip();
        let start = self.hash_start();
        let mut secondary_tip_queue: Vec<(String, Uuid)> = vec![];

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
            if remote.tip_secondary != tip {
                if remote.tip == start {
                    let mut last_id: Option<Uuid> = None;
                    for (_, msg) in self.messages.iter().take(8) {
                        last_id = Some(msg.id());
                        tx(remote.key.clone(), HeadCommand::Put(msg.clone()));
                    }
                    if let Some(last_id) = last_id {
                        secondary_tip_queue.push((remote.key.clone(), last_id));
                    }
                } else if let Some(uuid) = self.hashes.get_by_right(&remote.tip) {
                    let mut last_id: Option<Uuid> = None;
                    for (_, msg) in self.messages.range(uuid..).skip(1).take(8) {
                        last_id = Some(msg.id());
                        tx(remote.key.clone(), HeadCommand::Put(msg.clone()));
                    }
                    if let Some(last_id) = last_id {
                        secondary_tip_queue.push((remote.key.clone(), last_id));
                    }
                } else {
                    // In the long run we should probably make this more sophisticated,
                    // though so far I'm not sure how often this condition occurs,
                    // so instead of doing a complicated approach where we figure out
                    // which messages cause the heads to diverge, we just send the newest
                    // 4 messages and 12 random prior messages which should get the clients
                    // in sync somewhat quickly in most cases.
                    //
                    // In most cases they should be in-sync after one round because
                    // the most likely scenario is 2 clients creating messages at
                    // exactly the same time, especially in group chats though we
                    // might get messages from an offline client after a while which
                    // is why we send the 6 random old messages. One slight optimization
                    // we should try is instead of sending only the hash of the head
                    // we send 4-8 hashes to split the messages into "zones" then we
                    // could determine where the split started to occur and focus the
                    // sync on that zone.
                    for (_, msg) in self.messages.iter().rev().take(4) {
                        tx(remote.key.clone(), HeadCommand::Put(msg.clone()));
                    }

                    let mut rng = rand::rng();
                    let mut arr = self.messages.iter().collect::<Vec<_>>();
                    arr.shuffle(&mut rng);
                    for (_, msg) in arr.into_iter().take(12) {
                        tx(remote.key.clone(), HeadCommand::Put(msg.clone()));
                    }
                }
            };
            tx(remote.key.clone(), HeadCommand::Tip(tip.as_bytes().clone()));
            remote.cooldown_until = now + Duration::from_secs(300);
        }

        for (dest, uuid) in secondary_tip_queue.into_iter() {
            let tip_secondary = self.hash(uuid).expect("Couldn't hash secondary tip");
            if tip != tip_secondary {
                tx(
                    dest,
                    HeadCommand::TipSecondary(tip_secondary.as_bytes().clone()),
                );
            }
        }
    }
}
