use axum::extract::ws::{Message, WebSocket};
use hoshi_protocol::relay::{RelayErrorPacket, RelayPacket};

use crate::{
    ServerState,
    noise::accept_responder_handshake,
    state::{ConnectionIdentity, OutboundCommand},
};

const MAX_WS_FRAME_SIZE: usize = 128 * 1024;
const MAX_RELAY_PACKET_SIZE: usize = 64 * 1024;

pub(super) async fn relay_socket_loop(
    state: ServerState,
    mut socket: WebSocket,
    identity: ConnectionIdentity,
) {
    let Some(handshake_message) = socket.recv().await else {
        return;
    };

    let handshake_message = match handshake_message {
        Ok(Message::Binary(message)) => message,
        _ => {
            let _ = socket.send(Message::Close(None)).await;
            return;
        }
    };

    if handshake_message.len() > MAX_WS_FRAME_SIZE {
        let _ = socket.send(Message::Close(None)).await;
        return;
    }

    let mut noise_transport =
        match accept_responder_handshake(state.noise_static_private_key(), &handshake_message) {
            Ok((transport, response_message)) => {
                if socket
                    .send(Message::Binary(response_message.into()))
                    .await
                    .is_err()
                {
                    return;
                }
                transport
            }
            Err(_) => {
                let _ = socket.send(Message::Close(None)).await;
                return;
            }
        };

    let (outbound_tx, mut outbound_rx) = tokio::sync::mpsc::unbounded_channel();
    let session_id = state.register_session(&identity, outbound_tx);

    loop {
        tokio::select! {
            incoming = socket.recv() => {
                let Some(incoming) = incoming else {
                    break;
                };

                let message = match incoming {
                    Ok(message) => message,
                    Err(_) => break,
                };

                match message {
                    Message::Binary(ciphertext) => {
                        if ciphertext.len() > MAX_WS_FRAME_SIZE {
                            let _ = send_encrypted_error(&mut noise_transport, &mut socket, "malformed_packet", None).await;
                            continue;
                        }

                        let mut plaintext = vec![0_u8; ciphertext.len().max(1) + 1024];
                        let plaintext_len = match noise_transport.read_message(&ciphertext, &mut plaintext) {
                            Ok(len) => len,
                            Err(_) => break,
                        };

                        if plaintext_len > MAX_RELAY_PACKET_SIZE {
                            let _ = send_encrypted_error(&mut noise_transport, &mut socket, "malformed_packet", None).await;
                            continue;
                        }

                        let packet = match serde_json::from_slice::<RelayPacket>(&plaintext[..plaintext_len]) {
                            Ok(packet) => packet,
                            Err(_) => {
                                let _ = send_encrypted_error(&mut noise_transport, &mut socket, "malformed_packet", None).await;
                                continue;
                            }
                        };

                        if packet.recipient.trim().is_empty() {
                            let _ = send_encrypted_error(&mut noise_transport, &mut socket, "malformed_packet", None).await;
                            continue;
                        }

                        if let Err(err) = state.route_packet(packet.clone()) {
                            let _ = send_encrypted_error(
                                &mut noise_transport,
                                &mut socket,
                                err.code(),
                                Some(packet.recipient),
                            ).await;
                        }
                    }
                    Message::Close(_) => break,
                    Message::Ping(_) | Message::Pong(_) => {}
                    Message::Text(_) => {
                        let _ = send_encrypted_error(&mut noise_transport, &mut socket, "malformed_packet", None).await;
                    }
                }
            }
            outbound = outbound_rx.recv() => {
                let Some(outbound) = outbound else {
                    break;
                };

                match outbound {
                    OutboundCommand::Packet(packet) => {
                        if send_encrypted_packet(&mut noise_transport, &mut socket, &packet).await.is_err() {
                            break;
                        }
                    }
                    OutboundCommand::Close => {
                        let _ = socket.send(Message::Close(None)).await;
                        break;
                    }
                }
            }
        }
    }

    state.unregister_session_if_current(&identity, session_id);
    let _ = socket.send(Message::Close(None)).await;
}

async fn send_encrypted_packet(
    transport: &mut snow::TransportState,
    socket: &mut WebSocket,
    packet: &RelayPacket,
) -> Result<(), ()> {
    let payload = serde_json::to_vec(packet).map_err(|_| ())?;
    send_encrypted_bytes(transport, socket, &payload).await
}

async fn send_encrypted_error(
    transport: &mut snow::TransportState,
    socket: &mut WebSocket,
    code: &str,
    recipient: Option<String>,
) -> Result<(), ()> {
    let payload = serde_json::to_vec(&RelayErrorPacket {
        error: code.to_string(),
        recipient,
    })
    .map_err(|_| ())?;

    send_encrypted_bytes(transport, socket, &payload).await
}

async fn send_encrypted_bytes(
    transport: &mut snow::TransportState,
    socket: &mut WebSocket,
    payload: &[u8],
) -> Result<(), ()> {
    if payload.len() > MAX_RELAY_PACKET_SIZE {
        return Err(());
    }

    let mut ciphertext = vec![0_u8; payload.len() + 1024];
    let ciphertext_len = transport
        .write_message(payload, &mut ciphertext)
        .map_err(|_| ())?;

    socket
        .send(Message::Binary(
            ciphertext[..ciphertext_len].to_vec().into(),
        ))
        .await
        .map_err(|_| ())
}
