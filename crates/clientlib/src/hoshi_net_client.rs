use futures::{SinkExt, StreamExt};
use reqwest::header::USER_AGENT;
use reqwest_websocket::{Message, RequestBuilderExt};
use serde::{Deserialize, Serialize};
use std::{cell::RefCell, sync::mpsc};

use crate::{ChatMessage, RelayInfo, audio_chunk::AudioChunk, call::CallPartyStatus};

pub struct HoshiNetClient {
    relay_list: RefCell<Vec<RelayInfo>>,
    pipes: RefCell<Vec<WebSocketPipe>>,
    outbox: RefCell<Vec<HoshiMessage>>,
    public_key: RefCell<String>,
}

impl HoshiNetClient {
    pub fn new() -> Self {
        Self {
            relay_list: RefCell::new(vec![]),
            pipes: RefCell::new(vec![]),
            outbox: RefCell::new(vec![]),
            public_key: RefCell::new(String::new()),
        }
    }

    pub fn set_public_key(&self, key: String) {
        *self.public_key.borrow_mut() = key;
    }

    pub fn disconnect_all(&self) {
        self.pipes.borrow_mut().clear();
    }

    pub fn update_relays(&self, new_relays: &Vec<RelayInfo>) {
        let mut relay_list = self.relay_list.borrow_mut();
        relay_list.clear();
        for relay in new_relays {
            relay_list.push(relay.clone());
        }
    }

    pub fn send(&self, msg: HoshiMessage) {
        self.outbox.borrow_mut().push(msg);
    }

    pub fn step(&self) -> Vec<HoshiMessage> {
        // Open pipes for relays that don't have one yet
        {
            let relay_list = self.relay_list.borrow();
            let public_key = self.public_key.borrow().clone();
            let mut pipes = self.pipes.borrow_mut();
            for relay in relay_list.iter() {
                let already_connected = pipes.iter().any(|p| p.relay.url == relay.url);
                if !already_connected {
                    pipes.push(WebSocketPipe::new(relay.clone(), public_key.clone()));
                }
            }
        }

        // Drain outbox into all pipes, non-blocking
        {
            let outbox: Vec<HoshiMessage> = self.outbox.borrow_mut().drain(..).collect();
            let pipes = self.pipes.borrow();
            for msg in outbox {
                for pipe in pipes.iter() {
                    // ignore send errors, pipe might be dead
                    let _ = pipe.tx.send(msg.clone());
                }
            }
        }

        // Drain inbox from all pipes, non-blocking
        let mut received = vec![];
        {
            let pipes = self.pipes.borrow();
            for pipe in pipes.iter() {
                while let Ok(msg) = pipe.rx.try_recv() {
                    received.push(msg);
                }
            }
        }

        received
    }
}

pub struct WebSocketPipe {
    relay: RelayInfo,
    tx: tokio::sync::mpsc::UnboundedSender<HoshiMessage>,
    rx: mpsc::Receiver<HoshiMessage>,
}

impl WebSocketPipe {
    pub fn new(relay: RelayInfo, public_key: String) -> Self {
        let (tokio_tx, tokio_rx) = tokio::sync::mpsc::unbounded_channel::<HoshiMessage>();
        let (cli_tx, cli_rx) = mpsc::channel::<HoshiMessage>();

        {
            let relay = relay.clone();
            std::thread::spawn(move || {
                let mut rx = tokio_rx;
                let tx = cli_tx;
                let rt = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .unwrap();
                rt.block_on(async move {
                    let ws = match reqwest::Client::default()
                        .get(&relay.url)
                        .header("Authorization", format!("Bearer {}", public_key))
                        .header(USER_AGENT, format!("Hoshi Messenger {}", env!("CARGO_PKG_VERSION")))
                        .upgrade()
                        .send()
                        .await
                        .and_then(|r| Ok(r))
                    {
                        Ok(r) => match r.into_websocket().await {
                            Ok(ws) => ws,
                            Err(e) => {
                                eprintln!("WS upgrade failed for {}: {e}", relay.url);
                                return;
                            }
                        },
                        Err(e) => {
                            eprintln!("WS connect failed for {}: {e}", relay.url);
                            return;
                        }
                    };

                    let (mut sink, mut stream) = ws.split();

                    loop {
                        tokio::select! {
                            // relay -> client
                            msg = stream.next() => {
                                match msg {
                                    Some(Ok(Message::Text(text))) => {
                                        println!("WS Text: {text}");
                                    }
                                    Some(Ok(Message::Binary(bytes))) => {
                                        if let Ok(envelope) = rmp_serde::from_slice::<HoshiEnvelope>(&bytes) {
                                            if let Ok(msg) = rmp_serde::from_slice::<HoshiMessage>(&envelope.payload) {
                                                tx.send(msg).ok();
                                            }
                                        }
                                    }
                                    Some(Err(e)) => {
                                        eprintln!("WS error: {e:?}");
                                        break;
                                    }
                                    None => {
                                        eprintln!("WS closed by relay");
                                        break;
                                    }
                                    _ => {}
                                }
                            }
                            // client -> relay
                            msg = rx.recv() => {
                                match msg {
                                    Some(msg) => {
                                        if let Ok(payload) = rmp_serde::to_vec(&msg) {
                                            let envelope = HoshiEnvelope {
                                                recipient: msg.to_key.clone(),
                                                payload,
                                            };
                                            if let Ok(bytes) = rmp_serde::to_vec(&envelope) {
                                                let _ = sink.send(Message::Binary(bytes.into())).await;
                                            }
                                        }
                                    }
                                    None => {
                                        // channel closed, client gone
                                        break;
                                    }
                                }
                            }
                        }
                    }
                });
            });
        }

        Self {
            relay,
            rx: cli_rx,
            tx: tokio_tx,
        }
    }
}

/// This is the envelope that is sent to the relay, the idea is to later
/// use noise protocl to encrypt the payload, to keep things simple for
/// now just send it unencrypted in serialized form, once we get the
/// networking side stable we'll encrypt this, the idea is that the relay
/// only needs to know information in the Envelope, nothing more, it just
/// passes along some bytes
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct HoshiEnvelope {
    pub recipient: String,
    pub payload: Vec<u8>,
}

/// The actual messages used by clients, since every message must have a
/// sender/receiver we put it here in addition to an enum payload
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct HoshiMessage {
    pub from_key: String,
    pub to_key: String,
    pub payload: HoshiPayload,
}

impl HoshiMessage {
    pub fn new(from_key: String, to_key: String, payload: HoshiPayload) -> Self {
        Self {
            from_key,
            to_key,
            payload,
        }
    }
}

/// The payload, for now just Ping/Pong, we'll extend this later
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum HoshiPayload {
    Ping,
    Pong,
    RequestChatMessages,
    ChatMessage(ChatMessage),
    InviteToCall { from_key: String, id: String },
    UpdateCallStatus { id: String, status: CallPartyStatus },
    AudioChunk(AudioChunk),
}
