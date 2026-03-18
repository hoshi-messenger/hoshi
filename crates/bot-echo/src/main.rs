use std::{cell::RefCell, env::home_dir, rc::Rc, time::Duration};

use anyhow::Result;
use hoshi_clientlib::{CallPartyStatus, ChatMessage, HoshiClient};

mod loopback_interface;
use loopback_interface::LoopbackInterface;

fn main() -> Result<()> {
    let path = home_dir().unwrap_or("./".into());
    let path = path.join(".hoshi").join("bot-echo");
    let client = HoshiClient::new(Some(path))?;
    let msg_queue: Rc<RefCell<Vec<ChatMessage>>> = Rc::new(RefCell::new(vec![]));

    let interface = LoopbackInterface::new();
    client.set_audio_interface(Some(Box::new(interface)));

    {
        let msg_queue = msg_queue.clone();
        client.messages_watch("".to_string(), move |client, _filter, msgs| {
            let mut sorted = msgs.values().collect::<Vec<&ChatMessage>>();
            sorted.sort();
            if let Some(last) = sorted.last()
                && last.from != client.public_key()
            {
                let reply = ChatMessage::create(
                    client.public_key(),
                    last.from.to_string(),
                    format!(">> {}", last.content),
                );
                println!("Queing reply to {} with {}", &reply.to, &reply.content);
                msg_queue.borrow_mut().push(reply);
            }
        });
    }

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
        "Hoshi Echo Bot starting, public_key: {}",
        client.public_key()
    );

    loop {
        for msg in msg_queue.borrow_mut().drain(0..) {
            println!("Replying to {} with {}", &msg.to, &msg.content);
            if client.message_upsert(msg).is_err() {
                eprintln!("Error replying to message");
            };
        }
        client.step();
        std::thread::sleep(Duration::from_millis(32));
    }
}
