use std::{env::home_dir, time::Duration};

use anyhow::Result;
use hoshi_clientlib::{CallPartyStatus, HoshiClient};

mod jukebox;
use jukebox::JukeboxInterface;

fn main() -> Result<()> {
    let path = home_dir().unwrap_or("./".into());
    let path = path.join(".hoshi").join("bot-jukebox.sqlite3");
    let client = HoshiClient::new(Some(path))?;

    let music_library = home_dir().unwrap_or("./".into());
    let music_library = music_library.join("Music");

    let interface = JukeboxInterface::new(music_library);
    client.set_audio_interface(Some(Box::new(interface)));

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
        println!("Calls in-progress: {}", calls.len());
    });

    println!(
        "Hoshi Jukebox Bot starting, public_key: {}",
        client.public_key()
    );

    loop {
        client.step();
        std::thread::sleep(Duration::from_millis(4));
    }
}
