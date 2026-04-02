use blake3::Hash;
use hoshi_clientlib::{Store, StoreHead};
use serde::{Deserialize, Serialize};
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

#[test]
fn basic_head_memory_tests() {
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

    // First make sure that an insert triggers a new hash
    head_a.insert(msg_2.clone());
    assert_eq!(head_a.len(), 1);
    assert_ne!(head_a.hash_tip(), head_a2.hash_tip());
    head_a2.insert(msg_2.clone());
    // Same name and same content should result in same hash
    assert_eq!(head_a.hash_tip(), head_a2.hash_tip());
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

    // Insertion order shouldn't matter for the final hash, same messages, same hashes
    let mut head_a = StoreHead::<Dummy>::new("a".to_string(), None);
    let mut head_a2 = StoreHead::<Dummy>::new("a".to_string(), None);

    head_a.insert(msg_1.clone());
    head_a2.insert(msg_3.clone());
    assert_ne!(head_a.hash_tip(), head_a2.hash_tip());

    head_a.insert(msg_2.clone());
    head_a2.insert(msg_2.clone());
    assert_ne!(head_a.hash_tip(), head_a2.hash_tip());

    head_a.insert(msg_3.clone());
    head_a2.insert(msg_1.clone());
    assert_eq!(head_a.hash_tip(), head_a2.hash_tip());
}
