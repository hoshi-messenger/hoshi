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
    sync_stores(&mut a, &mut b, 8);
    assert_eq!(a.hash_tip(), b.hash_tip());
    assert_eq!(a.len(), 8);
    assert_eq!(a.len(), b.len());

    eprintln!(" == Phase 1 ==");
    for i in 8..16 {
        b.insert(Dummy::new(format!("{i}"), None));
    }
    sync_stores(&mut a, &mut b, 8);
    assert_eq!(a.hash_tip(), b.hash_tip());
    assert_eq!(a.len(), 16);
    assert_eq!(a.len(), b.len());

    eprintln!(" == Phase 2 ==");
    for i in 16..32 {
        a.insert(Dummy::new(format!("{i}"), None));
    }
    sync_stores(&mut a, &mut b, 64);
    assert_eq!(a.hash_tip(), b.hash_tip());
    assert_eq!(a.len(), 32);
    assert_eq!(a.len(), b.len());
}
