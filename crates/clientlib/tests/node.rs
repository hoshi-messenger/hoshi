use hoshi_clientlib::{HoshiNode, HoshiNodePayload, NodeStore};

#[test]
fn insert_and_get() {
    let mut store = NodeStore::new(None, String::new());
    store.insert(HoshiNode {
        from: "alice".into(),
        path: "/chat/room1/msg1".into(),
        payload: HoshiNodePayload::Message,
    });

    let node = store.get("/chat/room1/msg1").unwrap();
    assert_eq!(node.from, "alice");
    assert!(matches!(node.payload, HoshiNodePayload::Message));
    assert!(store.get("/chat/room1/msg2").is_none());
}

#[test]
fn children_returns_direct_only() {
    let mut store = NodeStore::new(None, String::new());
    store.insert(HoshiNode {
        from: "alice".into(),
        path: "/chat/room1/msg1".into(),
        payload: HoshiNodePayload::Message,
    });
    store.insert(HoshiNode {
        from: "alice".into(),
        path: "/chat/room1/msg1/text1".into(),
        payload: HoshiNodePayload::ChatText {
            content: "hello".into(),
        },
    });
    store.insert(HoshiNode {
        from: "alice".into(),
        path: "/chat/room1/msg1/text1/deep".into(),
        payload: HoshiNodePayload::ChatText {
            content: "nested".into(),
        },
    });
    store.insert(HoshiNode {
        from: "bob".into(),
        path: "/chat/room1/msg2".into(),
        payload: HoshiNodePayload::Message,
    });

    let room_children = store.children("/chat/room1");
    assert_eq!(room_children.len(), 2);
    assert_eq!(room_children[0].path, "/chat/room1/msg1");
    assert_eq!(room_children[1].path, "/chat/room1/msg2");

    let msg_children = store.children("/chat/room1/msg1");
    assert_eq!(msg_children.len(), 1);
    assert_eq!(msg_children[0].path, "/chat/room1/msg1/text1");
}

#[test]
fn edit_and_delete_via_children() {
    let mut store = NodeStore::new(None, String::new());
    store.insert(HoshiNode {
        from: "alice".into(),
        path: "/chat/room1/msg1".into(),
        payload: HoshiNodePayload::Message,
    });
    store.insert(HoshiNode {
        from: "alice".into(),
        path: "/chat/room1/msg1/aaa".into(),
        payload: HoshiNodePayload::ChatText {
            content: "original".into(),
        },
    });
    store.insert(HoshiNode {
        from: "alice".into(),
        path: "/chat/room1/msg1/bbb".into(),
        payload: HoshiNodePayload::ChatText {
            content: "edited".into(),
        },
    });

    let children = store.children("/chat/room1/msg1");
    assert_eq!(children.len(), 2);
    let latest = children.last().unwrap();
    assert!(matches!(
        &latest.payload,
        HoshiNodePayload::ChatText { content } if content == "edited"
    ));
}

#[test]
fn hash_leaf_node() {
    let mut store = NodeStore::new(None, String::new());
    store.insert(HoshiNode {
        from: "alice".into(),
        path: "/chat/room1/msg1".into(),
        payload: HoshiNodePayload::Message,
    });

    let h1 = store.hash("/chat/room1/msg1");
    let h2 = store.hash("/chat/room1/msg1");
    assert_eq!(h1, h2, "memoized hash should be stable");
}

#[test]
fn hash_changes_on_child_insert() {
    let mut store = NodeStore::new(None, String::new());
    store.insert(HoshiNode {
        from: "alice".into(),
        path: "/chat/room1/msg1".into(),
        payload: HoshiNodePayload::Message,
    });
    store.insert(HoshiNode {
        from: "alice".into(),
        path: "/chat/room1/msg1/aaa".into(),
        payload: HoshiNodePayload::ChatText {
            content: "hello".into(),
        },
    });

    let h_before = store.hash("/chat/room1/msg1");

    store.insert(HoshiNode {
        from: "alice".into(),
        path: "/chat/room1/msg1/bbb".into(),
        payload: HoshiNodePayload::ChatText {
            content: "edit".into(),
        },
    });

    let h_after = store.hash("/chat/room1/msg1");
    assert_ne!(h_before, h_after, "hash must change when children change");
}

#[test]
fn hash_propagates_up() {
    let mut store = NodeStore::new(None, String::new());
    store.insert(HoshiNode {
        from: "alice".into(),
        path: "/chat/room1/msg1".into(),
        payload: HoshiNodePayload::Message,
    });
    store.insert(HoshiNode {
        from: "alice".into(),
        path: "/chat/room1/msg1/aaa".into(),
        payload: HoshiNodePayload::ChatText {
            content: "hello".into(),
        },
    });

    let room_hash_before = store.hash("/chat/room1");

    store.insert(HoshiNode {
        from: "bob".into(),
        path: "/chat/room1/msg2".into(),
        payload: HoshiNodePayload::Message,
    });

    let room_hash_after = store.hash("/chat/room1");
    assert_ne!(
        room_hash_before, room_hash_after,
        "parent hash must change when subtree changes"
    );
}

#[test]
fn set_hash_without_data() {
    let mut store = NodeStore::new(None, String::new());
    let fake_hash = blake3::hash(b"remote subtree");
    store.set_hash("/chat/room1/msg1".into(), fake_hash);

    assert_eq!(store.get_hash("/chat/room1/msg1"), Some(fake_hash));
    assert!(store.get("/chat/room1/msg1").is_none());
}

// -- Persistence tests --

#[test]
fn persistent_get_survives_new_instance() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().to_path_buf();

    {
        let mut store = NodeStore::new(Some(root.clone()), String::new());
        store.insert(HoshiNode {
            from: "alice".into(),
            path: "/chat/room1/msg1".into(),
            payload: HoshiNodePayload::Message,
        });
        store.insert(HoshiNode {
            from: "alice".into(),
            path: "/chat/room1/msg1/aaa".into(),
            payload: HoshiNodePayload::ChatText {
                content: "hello".into(),
            },
        });
    }

    // Fresh store with empty BTreeMaps, should load from disk
    let mut store = NodeStore::new(Some(root), String::new());
    let node = store.get("/chat/room1/msg1").unwrap();
    assert_eq!(node.from, "alice");
    assert!(matches!(node.payload, HoshiNodePayload::Message));

    let node = store.get("/chat/room1/msg1/aaa").unwrap();
    assert!(matches!(
        &node.payload,
        HoshiNodePayload::ChatText { content } if content == "hello"
    ));
}

#[test]
fn persistent_children_survives_new_instance() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().to_path_buf();

    {
        let mut store = NodeStore::new(Some(root.clone()), String::new());
        store.insert(HoshiNode {
            from: "alice".into(),
            path: "/chat/room1/msg1".into(),
            payload: HoshiNodePayload::Message,
        });
        store.insert(HoshiNode {
            from: "bob".into(),
            path: "/chat/room1/msg2".into(),
            payload: HoshiNodePayload::Message,
        });
    }

    let mut store = NodeStore::new(Some(root), String::new());
    let children = store.children("/chat/room1");
    assert_eq!(children.len(), 2);
    assert_eq!(children[0].path, "/chat/room1/msg1");
    assert_eq!(children[1].path, "/chat/room1/msg2");
}

#[test]
fn persistent_hash_recomputed_after_restart() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().to_path_buf();

    let h;
    {
        let mut store = NodeStore::new(Some(root.clone()), String::new());
        store.insert(HoshiNode {
            from: "alice".into(),
            path: "/chat/room1/msg1".into(),
            payload: HoshiNodePayload::Message,
        });
        h = store.hash("/chat/room1/msg1");
    }

    // Hashes are not persisted, but recomputing should yield the same result
    let mut store = NodeStore::new(Some(root), String::new());
    assert_eq!(store.get_hash("/chat/room1/msg1"), None);
    assert_eq!(store.hash("/chat/room1/msg1"), h);
}

#[test]
fn persistent_hash_invalidated_on_insert() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().to_path_buf();

    let h_before;
    {
        let mut store = NodeStore::new(Some(root.clone()), String::new());
        store.insert(HoshiNode {
            from: "alice".into(),
            path: "/chat/room1/msg1".into(),
            payload: HoshiNodePayload::Message,
        });
        store.insert(HoshiNode {
            from: "alice".into(),
            path: "/chat/room1/msg1/aaa".into(),
            payload: HoshiNodePayload::ChatText {
                content: "hello".into(),
            },
        });
        h_before = store.hash("/chat/room1/msg1");
    }

    {
        let mut store = NodeStore::new(Some(root.clone()), String::new());
        store.insert(HoshiNode {
            from: "alice".into(),
            path: "/chat/room1/msg1/bbb".into(),
            payload: HoshiNodePayload::ChatText {
                content: "edit".into(),
            },
        });
    }

    let mut store = NodeStore::new(Some(root), String::new());
    // Parent hash should have been invalidated on disk
    let h_after = store.hash("/chat/room1/msg1");
    assert_ne!(h_before, h_after);
}
