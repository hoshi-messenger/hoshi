use hoshi_clientlib::{HoshiNode, HoshiNodePayload, NodeStore, chat_path};

#[test]
fn user_alias_set_and_get() {
    let pk = "aa".repeat(32);
    let mut store = NodeStore::new(None, pk.clone());
    assert!(store.user_alias_get(&pk).is_none());

    store.user_alias_set("Alice");
    assert_eq!(store.user_alias_get(&pk).unwrap(), "Alice");

    // Setting again overwrites (latest child wins)
    store.user_alias_set("Bob");
    assert_eq!(store.user_alias_get(&pk).unwrap(), "Bob");
}

#[test]
fn user_alias_set_skips_when_unchanged() {
    let pk = "aa".repeat(32);
    let mut store = NodeStore::new(None, pk.clone());

    assert!(store.user_alias_set("Alice"));
    let children_before = store.children(&format!("/user/{pk}")).len();

    // Setting the same alias again should be a no-op
    assert!(!store.user_alias_set("Alice"));
    let children_after = store.children(&format!("/user/{pk}")).len();

    assert_eq!(children_before, children_after);
}

#[test]
fn user_path_may_read_allows_anyone() {
    let own_pk = "aa".repeat(32);
    let other_pk = "bb".repeat(32);
    let store = NodeStore::new(None, own_pk.clone());

    assert!(store.may_read(&other_pk, &format!("/user/{own_pk}")));
    assert!(store.may_read(&other_pk, &format!("/user/{own_pk}/some-uuid")));
    assert!(store.may_read(&own_pk, &format!("/user/{own_pk}")));
}

#[test]
fn user_path_may_write_owner_only() {
    let own_pk = "aa".repeat(32);
    let other_pk = "bb".repeat(32);
    let store = NodeStore::new(None, own_pk.clone());

    let node = HoshiNode {
        from: own_pk.clone(),
        path: format!("/user/{own_pk}/some-uuid"),
        payload: HoshiNodePayload::Title("Alice".into()),
    };

    // Owner can write
    assert!(store.may_write(&own_pk, &node.path, &node));

    // Other cannot write
    assert!(!store.may_write(&other_pk, &node.path, &node));
}

#[test]
fn chat_may_read_still_works() {
    let a = "aa".repeat(32);
    let b = "bb".repeat(32);
    let c = "cc".repeat(32);
    let store = NodeStore::new(None, a.clone());

    let cp = chat_path(&a, &b);
    assert!(store.may_read(&b, &cp));
    assert!(!store.may_read(&c, &cp));
}
