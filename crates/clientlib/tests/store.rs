use std::{cell::RefCell, collections::HashMap, rc::Rc};

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
    fn id(&self) -> Uuid {
        self.id
    }

    fn hash(&self) -> Hash {
        blake3::Hasher::new()
            .update(self.id.as_bytes())
            .update(self.text.as_bytes())
            .finalize()
    }
}

impl Dummy {
    fn new(text: String, id: Option<Uuid>) -> Self {
        let id = id.unwrap_or_else(|| Uuid::now_v7());
        Self { id, text }
    }
}

fn concat_all(a: &mut StoreHead<Dummy>) -> String {
    a.get_all()
        .iter()
        .map(|(_, msg)| msg.text.clone())
        .collect::<Vec<_>>()
        .join("")
}

fn sync_stores<T: Store>(
    a: &mut StoreHead<T>,
    b: &mut StoreHead<T>,
    max_rounds: i32,
) -> (usize, usize) {
    // Make sure the 2 remotes know about each other
    a.remote_add("b".to_string(), None);
    b.remote_add("a".to_string(), None);
    // We need a place to store messages for the 2 to communicate via
    let mut inbox_a: Vec<HeadCommand<T>> = vec![];
    let mut inbox_b: Vec<HeadCommand<T>> = vec![];
    // Hard upper bound, syncing must succeed after 32 iterations
    let mut messages = 0;
    let mut rounds = 0;
    for _i in 0..max_rounds {
        rounds += 1;
        inbox_a.drain(0..).for_each(|msg| a.rx("b", msg));
        inbox_b.drain(0..).for_each(|msg| b.rx("a", msg));

        let mut messages_queued = inbox_a.len() + inbox_b.len();
        a.tx(|dest, msg| {
            assert_eq!(dest, "b");
            eprintln!("a->b = {:?}", &msg);
            messages_queued += 1;
            inbox_b.push(msg);
        });
        b.tx(|dest, msg| {
            assert_eq!(dest, "a");
            eprintln!("a<-b = {:?}", &msg);
            messages_queued += 1;
            inbox_a.push(msg);
        });
        if messages_queued == 0 {
            break;
        } else {
            messages += messages_queued;
        }
    }
    (rounds, messages)
}

fn sync_many_direct<T: Store>(
    stores: &mut HashMap<String, StoreHead<T>>,
    max_rounds: i32,
) -> (usize, usize) {
    let mut inbox: HashMap<String, Vec<(String, HeadCommand<T>)>> = HashMap::new();
    for name in stores.keys() {
        inbox.insert(name.to_string(), vec![]);
    }
    for (key, store) in stores.iter_mut() {
        for cur_key in inbox.keys() {
            if cur_key != key {
                store.remote_add(cur_key.to_string(), None);
            }
        }
    }

    let mut messages = 0;
    let mut rounds = 0;
    for _i in 0..max_rounds {
        rounds += 1;

        for (key, store) in stores.iter_mut() {
            let inbox = inbox.get_mut(key).unwrap();
            inbox
                .drain(0..)
                .for_each(|(from, msg)| store.rx(&from, msg));
        }

        let mut messages_queued = 0;
        for (key, store) in stores.iter_mut() {
            store.tx(|dest, msg| {
                assert_ne!(key, &dest);
                messages_queued += 1;
                let inbox = inbox.get_mut(&dest).unwrap();
                inbox.push((key.to_string(), msg));
            });
        }

        if messages_queued == 0 {
            break;
        } else {
            messages += messages_queued;
        }
    }
    (rounds, messages)
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

    let (rounds, messages) = sync_stores(&mut a, &mut b, 16);
    eprint!("Rounds: {}, Messages: {}", rounds, messages);
    assert!(rounds < 8);
    assert!(messages < 16);
    assert_eq!(a.hash_tip(), b.hash_tip());

    for i in 1..8 {
        a.insert(Dummy::new(format!("{i}"), None));
    }
    assert_ne!(a.hash_tip(), b.hash_tip());
    let (rounds, messages) = sync_stores(&mut a, &mut b, 16);
    eprintln!("Rounds: {}, Messages: {}", rounds, messages);
    assert!(rounds < 8);
    assert!(messages < 16);
    assert_eq!(a.hash_tip(), b.hash_tip());
    assert_eq!(a.len(), 8);
    assert_eq!(a.len(), b.len());

    for i in 8..16 {
        b.insert(Dummy::new(format!("{i}"), None));
    }
    let (rounds, messages) = sync_stores(&mut a, &mut b, 16);
    eprintln!("Rounds: {}, Messages: {}", rounds, messages);
    assert!(rounds < 8);
    assert!(messages < 24);
    assert_eq!(a.hash_tip(), b.hash_tip());
    assert_eq!(a.len(), 16);
    assert_eq!(a.len(), b.len());

    for i in 16..32 {
        a.insert(Dummy::new(format!("{i}"), None));
    }
    let (rounds, messages) = sync_stores(&mut a, &mut b, 16);
    eprintln!("Rounds: {}, Messages: {}", rounds, messages);
    assert!(rounds < 8);
    assert!(messages < 24);
    assert_eq!(a.hash_tip(), b.hash_tip());
    assert_eq!(a.len(), 32);
    assert_eq!(a.len(), b.len());

    for i in 32..256 {
        a.insert(Dummy::new(format!("{i}"), None));
    }
    let (rounds, messages) = sync_stores(&mut a, &mut b, 128);
    eprintln!("Rounds: {}, Messages: {}", rounds, messages);
    assert!(rounds < 96);
    assert!(messages < 512);
    assert_eq!(a.hash_tip(), b.hash_tip());
    assert_eq!(a.len(), 256);
    assert_eq!(a.len(), b.len());

    a.remote_drop("b");
    a.remote_add("b".to_string(), None);
    b.remote_drop("a");
    b.remote_add("a".to_string(), None);
    let (rounds, messages) = sync_stores(&mut a, &mut b, 128);
    eprintln!("Rounds: {}, Messages: {}", rounds, messages);
    assert!(rounds < 4);
    assert!(messages < 32);
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
    let (rounds, messages) = sync_stores(&mut a, &mut b, 2);
    eprintln!("Rounds: {}, Messages: {}", rounds, messages);
    assert!(messages < 4);
    assert_eq!(a.hash_tip(), b.hash_tip());

    let mut full_message: Vec<String> = vec!["0".to_string()];
    for i in 1..16 {
        let text = format!("{i}");
        full_message.push(text.clone());
        a.insert(Dummy::new(text, None));
    }
    for i in 16..32 {
        let text = format!("{i}");
        full_message.push(text.clone());
        b.insert(Dummy::new(text, None));
    }
    assert_ne!(a.hash_tip(), b.hash_tip());
    let (rounds, messages) = sync_stores(&mut a, &mut b, 32);
    eprintln!("Rounds: {}, Messages: {}", rounds, messages);
    assert!(rounds < 24);
    assert!(messages < 1024);
    assert_eq!(a.hash_tip(), b.hash_tip());
    assert_eq!(a.len(), 32);
    assert_eq!(a.len(), b.len());
    assert_eq!(full_message.join(""), concat_all(&mut a));

    for i in 32..128 {
        let text = format!("{i}");
        full_message.push(text.clone());
        let msg = Dummy::new(text, None);
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
    let (rounds, messages) = sync_stores(&mut a, &mut b, 128);
    eprintln!("Rounds: {}, Messages: {}", rounds, messages);
    assert!(messages < 4096);
    assert_eq!(a.len(), 128);
    assert_eq!(a.hash_tip(), b.hash_tip());
    assert_eq!(a.len(), b.len());
    assert_eq!(full_message.join(""), concat_all(&mut a));
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
    let (rounds, messages) = sync_many_direct(&mut stores, 4);
    eprintln!("Rounds: {}, Messages: {}", rounds, messages);
    assert!(messages < 256);
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

    stores
        .get_mut("a")
        .unwrap()
        .insert(Dummy::new("4".to_string(), None));
    let (rounds, messages) = sync_many_direct(&mut stores, 4);
    eprintln!("Rounds: {}, Messages: {}", rounds, messages);
    assert!(messages < 32);
    assert_eq!(concat_all(stores.get_mut("a").unwrap()), "01234");
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

    for i in 0..12 {
        stores
            .get_mut("a")
            .unwrap()
            .insert(Dummy::new(i.to_string(), None));
    }
    for i in 12..24 {
        let c = char::from_u32(('b' as u32) + (i - 12)).unwrap().to_string();
        stores
            .get_mut(&c)
            .unwrap()
            .insert(Dummy::new(i.to_string(), None));
    }

    assert_eq!(stores.get("a").unwrap().len(), 12);
    assert_eq!(stores.get("b").unwrap().len(), 1);
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
    let (rounds, messages) = sync_many_direct(&mut stores, 32);
    eprintln!("Rounds: {}, Messages: {}", rounds, messages);
    assert!(messages < 256 * 256);
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
    assert_eq!(stores.get("z").unwrap().len(), 24);
}

#[test]
fn watcher_tests() {
    let mut a = StoreHead::<Dummy>::new("t".to_string(), None);
    let mut b = StoreHead::<Dummy>::new("t".to_string(), None);

    let acc = Rc::new(RefCell::new("".to_string()));
    let watcher = {
        let acc = acc.clone();
        a.watcher_add(move |msgs| {
            let new_acc = msgs
                .iter()
                .map(|m| m.1.text.to_string())
                .collect::<Vec<_>>()
                .join("");
            acc.replace(new_acc);
        })
    };
    assert_eq!(a.watcher_len(), 1);
    assert_eq!(acc.borrow().clone(), "");

    a.insert(Dummy::new("0".to_string(), None));
    b.insert(Dummy::new("1".to_string(), None));
    b.insert(Dummy::new("2".to_string(), None));
    b.insert(Dummy::new("a".to_string(), None));
    assert_ne!(a.hash_tip(), b.hash_tip());
    assert_eq!(a.watcher_len(), 1);
    assert_eq!(acc.borrow().clone(), "0");

    let (rounds, messages) = sync_stores(&mut a, &mut b, 16);
    eprint!("Rounds: {}, Messages: {}", rounds, messages);
    assert!(rounds < 8);
    assert!(messages < 32);
    assert_eq!(a.hash_tip(), b.hash_tip());
    assert_eq!(a.watcher_len(), 1);
    assert_eq!(acc.borrow().clone(), "012a");

    drop(watcher);
    a.insert(Dummy::new("b".to_string(), None));
    assert_eq!(a.watcher_len(), 0);
    assert_eq!(acc.borrow().clone(), "012a");

    let _watcher = {
        let acc = acc.clone();
        a.watcher_add(move |msgs| {
            let new_acc = msgs
                .iter()
                .map(|m| m.1.text.to_string())
                .collect::<Vec<_>>()
                .join("");
            acc.replace(new_acc);
        })
    };
    assert_eq!(a.watcher_len(), 1);
    assert_eq!(acc.borrow().clone(), "012ab");
}
