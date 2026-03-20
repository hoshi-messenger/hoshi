use std::{cell::RefCell, env::home_dir, sync::mpsc, time::Duration};

use anyhow::Result;
use hoshi_clientlib::{CallPartyStatus, ChatMessage, HoshiClient};

mod jukebox;
use jukebox::JukeboxInterface;

fn main() -> Result<()> {
    let path = home_dir().unwrap_or("./".into());
    let path = path.join(".hoshi").join("bot-jukebox");
    let client = HoshiClient::new(Some(path))?;
    client.set_user_alias("Jukebox");

    let music_library = home_dir().unwrap_or("./".into());
    let music_library = music_library.join("Music");

    let (notify_tx, notify_rx) = mpsc::channel();
    let interface = JukeboxInterface::new(music_library, notify_tx);
    client.set_audio_interface(Some(Box::new(interface)));

    let active_calls = RefCell::new(0);
    client.calls_watch(move |client, calls| {
        let public_key = client.public_key();
        for call in calls.iter() {
            if matches!(call.get_status(&public_key), Some(CallPartyStatus::Ringing)) {
                if client.call_accept(call.id()).is_err() {
                    eprintln!(
                        "Couldn't accept call: {} with parties: {}",
                        call.id(),
                        call.get_call_label(client.own_contact())
                    );
                } else {
                    println!(
                        "Accepted call ({}) from: {}",
                        call.id(),
                        call.get_call_label(client.own_contact())
                    );
                }
            }
        }
        let mut active_calls = active_calls.borrow_mut();
        if *active_calls != calls.len() {
            println!("Active calls: {}", calls.len());
            *active_calls = calls.len();
        }
    });

    println!(
        "Hoshi Jukebox Bot starting, public_key: {}",
        client.public_key()
    );

    loop {
        client.step();
        while let Ok(notification) = notify_rx.try_recv() {
            let content = format!("Now playing: {}", notification.filename);
            for recipient in &notification.recipients {
                let msg = ChatMessage::create(
                    notification.from.clone(),
                    recipient.clone(),
                    content.clone(),
                );
                if let Err(e) = client.message_upsert(msg) {
                    eprintln!("Failed to send song notification: {e}");
                }
            }
        }
        std::thread::sleep(Duration::from_millis(32));
    }
}
