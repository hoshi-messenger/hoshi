use std::collections::HashMap;

use blake3::Hash;
use hoshi_clientlib::{HeadCommand, Store, StoreHead};
use serde::{Deserialize, Serialize};
use tempfile::tempdir;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Dummy {
    pub id: Uuid,
    pub text: String,
}

impl Store for Dummy {
    fn msg_id(&self) -> Uuid {
        self.id
    }

    fn msg_hash(&self) -> Hash {
        let mut hasher = blake3::Hasher::new();
        hasher.update(self.id.as_bytes());
        hasher.update(self.text.as_bytes());
        hasher.finalize()
    }
}

impl Dummy {
    fn new(text: String, id: Option<Uuid>) -> Self {
        let id = id.unwrap_or_else(|| Uuid::now_v7());

        Self { id, text }
    }
}

fn sync_stores<T: Store>(a: &mut StoreHead<T>, b: &mut StoreHead<T>, max_rounds: i32) {
    // Make sure the 2 remotes know about each other
    a.add_remote("b".to_string(), None);
    b.add_remote("a".to_string(), None);
    // We need a place to store messages for the 2 to communicate via
    let mut inbox_a: Vec<HeadCommand<T>> = vec![];
    let mut inbox_b: Vec<HeadCommand<T>> = vec![];
    // Hard upper bound, syncing must succeed after 32 iterations
    for _i in 0..max_rounds {
        inbox_a.drain(0..).for_each(|msg| a.rx("b", msg));
        inbox_b.drain(0..).for_each(|msg| b.rx("a", msg));
        a.tx(|dest, msg| {
            assert_eq!(dest, "b");
            eprintln!("a->b = {:?}", &msg);
            inbox_b.push(msg);
        });
        b.tx(|dest, msg| {
            assert_eq!(dest, "a");
            eprintln!("a<-b = {:?}", &msg);
            inbox_a.push(msg);
        });
    }
}

fn sync_many_direct<T: Store>(stores: &mut HashMap<String, StoreHead<T>>, max_rounds: i32) {
    let mut inbox: HashMap<String, Vec<(String, HeadCommand<T>)>> = HashMap::new();
    for name in stores.keys() {
        inbox.insert(name.to_string(), vec![]);
    }
    for (key, store) in stores.iter_mut() {
        for cur_key in inbox.keys() {
            if cur_key != key {
                store.add_remote(cur_key.to_string(), None);
            }
        }
    }

    for _i in 0..max_rounds {
        for (key, store) in stores.iter_mut() {
            let inbox = inbox.get_mut(key).unwrap();
            inbox
                .drain(0..)
                .for_each(|(from, msg)| store.rx(&from, msg));
        }

        for (key, store) in stores.iter_mut() {
            store.tx(|dest, msg| {
                assert_ne!(key, &dest);
                let inbox = inbox.get_mut(&dest).unwrap();
                inbox.push((key.to_string(), msg));
            });
        }
    }
}

#[test]
fn basic_head_tests() {
    let mut head_a = StoreHead::<Dummy>::new("a".to_string(), None);
    let mut head_a2 = StoreHead::<Dummy>::new("a".to_string(), None);
    let mut head_b = StoreHead::<Dummy>::new("b".to_string(), None);
    let msg_1 = Dummy::new("1".to_string(), None);
    let msg_2 = Dummy::new("2".to_string(), None);
    let msg_3 = Dummy::new("3".to_string(), None);

    // First make sure that all heads are empty
    assert_eq!(head_a.len(), 0);
    assert_eq!(head_a2.len(), 0);
    assert_eq!(head_b.len(), 0);
    // Every hash chain starts with the name hashed, which is why a==a2 and a!=b
    assert_eq!(head_a.hash_tip(), head_a2.hash_tip());
    assert_ne!(head_a.hash_tip(), head_b.hash_tip());

    // Now make sure that an insert triggers a new hash
    head_a.insert(msg_2.clone());
    assert_eq!(head_a.len(), 1);
    assert_ne!(head_a.hash_tip(), head_a2.hash_tip());
    head_a2.insert(msg_2.clone());
    // Same name and same content should result in same hash
    assert_eq!(head_a.hash_tip(), head_a2.hash_tip());
    // Inserting the same message twice shouldn't change the hash/count
    head_a.insert(msg_2.clone());
    assert_eq!(head_a.hash_tip(), head_a2.hash_tip());
    assert_eq!(head_a.len(), 1);

    head_b.insert(msg_2.clone());
    // Different name should trigger a different hash, even if messages are the same
    assert_ne!(head_a.hash_tip(), head_b.hash_tip());

    // Just making sure that another entry changes the tip hash
    head_a.insert(msg_3.clone());
    assert_eq!(head_a.len(), 2);
    assert_ne!(head_a.hash_tip(), head_a2.hash_tip());
    head_a2.insert(msg_3.clone());
    assert_eq!(head_a.hash_tip(), head_a2.hash_tip());

    // Make sure that when a message gets prepended hashes in the middle change
    let old_hash_a = head_a.hash(msg_2.id).unwrap();
    head_a.insert(msg_1.clone());
    let new_hash_a = head_a.hash(msg_2.id).unwrap();
    assert_ne!(old_hash_a, new_hash_a);
}

#[test]
fn head_insertion_order_mustnt_matter() {
    // Insertion order shouldn't matter for the final hash, same messages, same hashes
    let mut head_a = StoreHead::<Dummy>::new("a".to_string(), None);
    let mut head_a2 = StoreHead::<Dummy>::new("a".to_string(), None);
    let msg_1 = Dummy::new("1".to_string(), None);
    let msg_2 = Dummy::new("2".to_string(), None);
    let msg_3 = Dummy::new("3".to_string(), None);

    head_a.insert(msg_1.clone());
    head_a2.insert(msg_3.clone());
    assert_ne!(head_a.hash_tip(), head_a2.hash_tip());

    head_a.insert(msg_2.clone());
    head_a2.insert(msg_2.clone());
    assert_ne!(head_a.hash_tip(), head_a2.hash_tip());

    head_a.insert(msg_3.clone());
    head_a2.insert(msg_1.clone());
    assert_eq!(head_a.hash_tip(), head_a2.hash_tip());

    // Make sure that we generate proper Uuid'ss
    head_a.insert(Dummy::new("4".to_string(), None));
    head_a2.insert(Dummy::new("4".to_string(), None));
    assert_ne!(head_a.hash_tip(), head_a2.hash_tip());
}

#[test]
fn head_persistence() {
    let tmp = tempdir().unwrap();
    let mut head = StoreHead::<Dummy>::new("a".to_string(), Some(tmp.path()));
    let hash = head.hash_tip();
    assert_eq!(head.len(), 0);

    let mut head = StoreHead::<Dummy>::new("a".to_string(), Some(tmp.path()));
    assert_eq!(hash, head.hash_tip());

    head.insert(Dummy::new("1".to_string(), None));
    let hash = head.hash_tip();
    // Explicitly dropping head_a to ensure everything is written to disk
    drop(head);
    let mut head = StoreHead::<Dummy>::new("a".to_string(), Some(tmp.path()));
    assert_eq!(head.len(), 1);
    assert_eq!(hash, head.hash_tip());

    // Insert a couple of messages and make sure the hash stays the same
    for i in 2..101 {
        head.insert(Dummy::new(format!("{i}"), None));
    }
    let hash = head.hash_tip();
    drop(head);
    let mut head = StoreHead::<Dummy>::new("a".to_string(), Some(tmp.path()));
    assert_eq!(head.len(), 100);
    assert_eq!(hash, head.hash_tip());
}

#[test]
fn basic_sync() {
    // These are rather simple, we just test that if one client has newer
    // messages than another one we sync them, basically the best-case scenario
    // where we can just fast-forward
    let mut a = StoreHead::<Dummy>::new("t".to_string(), None);
    let mut b = StoreHead::<Dummy>::new("t".to_string(), None);

    a.insert(Dummy::new("0".to_string(), None));
    assert_ne!(a.hash_tip(), b.hash_tip());

    sync_stores(&mut a, &mut b, 2);
    assert_eq!(a.hash_tip(), b.hash_tip());

    eprintln!(" == Phase 0 ==");
    for i in 1..8 {
        a.insert(Dummy::new(format!("{i}"), None));
    }
    assert_ne!(a.hash_tip(), b.hash_tip());
    sync_stores(&mut a, &mut b, 4);
    assert_eq!(a.hash_tip(), b.hash_tip());
    assert_eq!(a.len(), 8);
    assert_eq!(a.len(), b.len());

    eprintln!(" == Phase 1 ==");
    for i in 8..16 {
        b.insert(Dummy::new(format!("{i}"), None));
    }
    sync_stores(&mut a, &mut b, 4);
    assert_eq!(a.hash_tip(), b.hash_tip());
    assert_eq!(a.len(), 16);
    assert_eq!(a.len(), b.len());

    eprintln!(" == Phase 2 ==");
    for i in 16..32 {
        a.insert(Dummy::new(format!("{i}"), None));
    }
    sync_stores(&mut a, &mut b, 16);
    assert_eq!(a.hash_tip(), b.hash_tip());
    assert_eq!(a.len(), 32);
    assert_eq!(a.len(), b.len());

    eprintln!(" == Phase 3 ==");
    for i in 32..256 {
        a.insert(Dummy::new(format!("{i}"), None));
    }
    sync_stores(&mut a, &mut b, 64);
    assert_eq!(a.hash_tip(), b.hash_tip());
    assert_eq!(a.len(), 256);
    assert_eq!(a.len(), b.len());
}

#[test]
fn complicated_sync() {
    // Here we test some slightly more complicated sync scenarios, mainly by
    // making sure that if both a and b have new messages the other doesn't know
    // about that they still converge after exchanging a couple of sync messages
    let mut a = StoreHead::<Dummy>::new("t".to_string(), None);
    let mut b = StoreHead::<Dummy>::new("t".to_string(), None);

    a.insert(Dummy::new("0".to_string(), None));
    assert_ne!(a.hash_tip(), b.hash_tip());
    sync_stores(&mut a, &mut b, 2);
    assert_eq!(a.hash_tip(), b.hash_tip());

    eprintln!(" == Phase 0 ==");
    for i in 1..16 {
        a.insert(Dummy::new(format!("{i}"), None));
    }
    for i in 16..32 {
        b.insert(Dummy::new(format!("{i}"), None));
    }
    assert_ne!(a.hash_tip(), b.hash_tip());
    sync_stores(&mut a, &mut b, 16);
    assert_eq!(a.hash_tip(), b.hash_tip());
    assert_eq!(a.len(), 32);
    assert_eq!(a.len(), b.len());

    for i in 32..256 {
        let msg = Dummy::new(format!("{i}"), None);
        match i & 3 {
            0 => a.insert(msg.clone()),
            1 => b.insert(msg.clone()),
            2 => a.insert(msg.clone()),
            _ => {
                a.insert(msg.clone());
                b.insert(msg.clone())
            }
        };
    }
    assert_ne!(a.hash_tip(), b.hash_tip());
    sync_stores(&mut a, &mut b, 256);
    assert_eq!(a.len(), 256);
    assert_eq!(a.hash_tip(), b.hash_tip());
    assert_eq!(a.len(), b.len());
}

#[test]
fn complicated_sync_many() {
    // Here we test some slightly more complicated sync scenarios, mainly by
    // making sure that if both a and b have new messages the other doesn't know
    // about that they still converge after exchanging a couple of sync messages
    let mut stores = HashMap::<String, StoreHead<Dummy>>::new();
    stores.insert(
        "a".to_string(),
        StoreHead::<Dummy>::new("t".to_string(), None),
    );
    stores.insert(
        "b".to_string(),
        StoreHead::<Dummy>::new("t".to_string(), None),
    );
    stores.insert(
        "c".to_string(),
        StoreHead::<Dummy>::new("t".to_string(), None),
    );
    stores.insert(
        "d".to_string(),
        StoreHead::<Dummy>::new("t".to_string(), None),
    );

    stores
        .get_mut("a")
        .unwrap()
        .insert(Dummy::new("0".to_string(), None));
    stores
        .get_mut("b")
        .unwrap()
        .insert(Dummy::new("1".to_string(), None));
    stores
        .get_mut("c")
        .unwrap()
        .insert(Dummy::new("2".to_string(), None));
    stores
        .get_mut("d")
        .unwrap()
        .insert(Dummy::new("3".to_string(), None));
    assert_eq!(stores.get("a").unwrap().len(), 1);
    assert_eq!(stores.get("b").unwrap().len(), 1);
    assert_eq!(stores.get("c").unwrap().len(), 1);
    assert_eq!(stores.get("d").unwrap().len(), 1);
    assert_ne!(
        stores.get_mut("a").unwrap().hash_tip(),
        stores.get_mut("b").unwrap().hash_tip()
    );
    assert_ne!(
        stores.get_mut("b").unwrap().hash_tip(),
        stores.get_mut("c").unwrap().hash_tip()
    );
    assert_ne!(
        stores.get_mut("c").unwrap().hash_tip(),
        stores.get_mut("d").unwrap().hash_tip()
    );
    sync_many_direct(&mut stores, 4);
    assert_eq!(
        stores.get_mut("a").unwrap().hash_tip(),
        stores.get_mut("b").unwrap().hash_tip()
    );
    assert_eq!(
        stores.get_mut("b").unwrap().hash_tip(),
        stores.get_mut("c").unwrap().hash_tip()
    );
    assert_eq!(
        stores.get_mut("c").unwrap().hash_tip(),
        stores.get_mut("d").unwrap().hash_tip()
    );
    assert_eq!(stores.get("a").unwrap().len(), 4);
}

#[test]
fn complicated_sync_many_many() {
    // Here we test some slightly more complicated sync scenarios, mainly by
    // making sure that if both a and b have new messages the other doesn't know
    // about that they still converge after exchanging a couple of sync messages
    let mut stores = HashMap::<String, StoreHead<Dummy>>::new();
    for c in 'a'..='z' {
        stores.insert(
            c.to_string(),
            StoreHead::<Dummy>::new("t".to_string(), None),
        );
    }

    for i in 0..32 {
        stores
            .get_mut("a")
            .unwrap()
            .insert(Dummy::new(i.to_string(), None));
    }
    for i in 32..64 {
        stores
            .get_mut("b")
            .unwrap()
            .insert(Dummy::new(i.to_string(), None));
    }

    assert_eq!(stores.get("a").unwrap().len(), 32);
    assert_eq!(stores.get("b").unwrap().len(), 32);
    assert_eq!(stores.get("c").unwrap().len(), 0);
    assert_eq!(stores.get("d").unwrap().len(), 0);
    assert_eq!(stores.get("z").unwrap().len(), 0);
    assert_ne!(
        stores.get_mut("a").unwrap().hash_tip(),
        stores.get_mut("b").unwrap().hash_tip()
    );
    assert_ne!(
        stores.get_mut("b").unwrap().hash_tip(),
        stores.get_mut("c").unwrap().hash_tip()
    );
    assert_ne!(
        stores.get_mut("b").unwrap().hash_tip(),
        stores.get_mut("d").unwrap().hash_tip()
    );
    assert_ne!(
        stores.get_mut("b").unwrap().hash_tip(),
        stores.get_mut("z").unwrap().hash_tip()
    );
    sync_many_direct(&mut stores, 16);
    assert_eq!(
        stores.get_mut("a").unwrap().hash_tip(),
        stores.get_mut("b").unwrap().hash_tip()
    );
    assert_eq!(
        stores.get_mut("b").unwrap().hash_tip(),
        stores.get_mut("c").unwrap().hash_tip()
    );
    assert_eq!(
        stores.get_mut("c").unwrap().hash_tip(),
        stores.get_mut("d").unwrap().hash_tip()
    );
    assert_eq!(
        stores.get_mut("d").unwrap().hash_tip(),
        stores.get_mut("z").unwrap().hash_tip()
    );
    assert_eq!(stores.get("z").unwrap().len(), 64);
}
