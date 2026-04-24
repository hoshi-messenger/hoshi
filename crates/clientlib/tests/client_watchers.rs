use std::{cell::RefCell, rc::Rc};

use anyhow::Result;
use hoshi_clientlib::{Call, ChatMessage, Contact, HoshiClient};

fn key(byte: u8) -> String {
    format!("{byte:02x}").repeat(32)
}

#[test]
fn unified_watch_handles_unsubscribe_on_drop() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let client = HoshiClient::new(Some(dir.path().to_path_buf()))?;
    let peer_key = key(0x22);

    let contact_events = Rc::new(RefCell::new(Vec::<usize>::new()));
    let message_events = Rc::new(RefCell::new(Vec::<usize>::new()));
    let call_events = Rc::new(RefCell::new(Vec::<usize>::new()));

    let contacts_watch = {
        let contact_events = Rc::clone(&contact_events);
        client.contacts_watch(move |_, contacts| {
            contact_events.borrow_mut().push(contacts.len());
        })
    };
    let messages_watch = {
        let message_events = Rc::clone(&message_events);
        client.messages_watch(String::new(), move |_, _, messages| {
            message_events.borrow_mut().push(messages.len());
        })
    };
    let calls_watch = {
        let call_events = Rc::clone(&call_events);
        client.calls_watch(move |_, calls| {
            call_events.borrow_mut().push(calls.len());
        })
    };

    assert_eq!(&*contact_events.borrow(), &[0]);
    assert!(message_events.borrow().is_empty());
    assert_eq!(&*call_events.borrow(), &[0]);

    client.contact_upsert(Contact::new(peer_key.clone()))?;
    client.step();
    assert_eq!(&*contact_events.borrow(), &[0, 1]);

    let msg = ChatMessage::create(client.public_key(), peer_key.clone(), "hello".to_string());
    client.message_upsert(msg)?;
    client.step();
    assert_eq!(&*message_events.borrow(), &[1]);

    client.call_start(vec![Contact::new(peer_key.clone())]);
    assert_eq!(&*call_events.borrow(), &[0, 1]);

    drop(contacts_watch);
    drop(messages_watch);
    drop(calls_watch);

    client.contact_upsert(Contact::new(key(0x33)))?;
    let msg = ChatMessage::create(client.public_key(), peer_key, "again".to_string());
    client.message_upsert(msg)?;
    client.call_start(vec![Contact::new(key(0x44))]);
    client.step();

    assert_eq!(&*contact_events.borrow(), &[0, 1]);
    assert_eq!(&*message_events.borrow(), &[1]);
    assert_eq!(&*call_events.borrow(), &[0, 1]);
    Ok(())
}

#[test]
fn store_head_backed_state_reloads_without_cached_views() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let peer_key = key(0x55);

    {
        let client = HoshiClient::new(Some(dir.path().to_path_buf()))?;
        client.contact_upsert(Contact::new(peer_key.clone()))?;
        client.set_user_alias("Alice");
        client.message_upsert(ChatMessage::create(
            client.public_key(),
            peer_key.clone(),
            "persist me".to_string(),
        ))?;
        client.step();
    }

    let client = HoshiClient::new(Some(dir.path().to_path_buf()))?;
    assert!(client.contact_get(&peer_key).is_some());
    assert_eq!(
        client.user_alias(&client.public_key()).as_deref(),
        Some("Alice")
    );

    let chat_id = ChatMessage::calc_chat_id(&client.public_key(), &peer_key);
    let last_message = client
        .last_message(&chat_id)
        .expect("missing reloaded chat message");
    assert_eq!(last_message.content, "persist me");
    Ok(())
}

#[test]
fn calls_watch_returns_drop_handle() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let client = HoshiClient::new(Some(dir.path().to_path_buf()))?;

    let events = Rc::new(RefCell::new(Vec::<usize>::new()));
    let watch = {
        let events = Rc::clone(&events);
        client.calls_watch(move |_, calls: &Vec<Call>| {
            events.borrow_mut().push(calls.len());
        })
    };

    client.call_start(vec![Contact::new(key(0x66))]);
    assert_eq!(&*events.borrow(), &[0, 1]);

    drop(watch);
    client.call_start(vec![Contact::new(key(0x77))]);
    assert_eq!(&*events.borrow(), &[0, 1]);
    Ok(())
}
