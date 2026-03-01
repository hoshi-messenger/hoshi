use hoshi_protocol::{control_plane::ClientType, relay::RelayPacket};
use std::sync::atomic::Ordering;

use tokio::sync::mpsc;

use super::ServerState;

#[derive(Debug, Clone)]
pub struct ConnectionIdentity {
    pub guid: String,
    pub client_type: ClientType,
}

#[derive(Debug, Clone)]
pub enum OutboundCommand {
    Packet(RelayPacket),
    Close,
}

#[derive(Debug, Clone)]
pub(super) struct SessionHandle {
    pub(super) session_id: u64,
    pub(super) tx: mpsc::UnboundedSender<OutboundCommand>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RouteError;

impl RouteError {
    pub fn code(self) -> &'static str {
        "recipient_not_connected"
    }
}

impl ServerState {
    pub fn register_session(
        &self,
        identity: &ConnectionIdentity,
        tx: mpsc::UnboundedSender<OutboundCommand>,
    ) -> u64 {
        let session_id = self.next_session_id.fetch_add(1, Ordering::Relaxed) + 1;

        let new_handle = SessionHandle { session_id, tx };

        self.guid_sessions
            .entry(identity.guid.clone())
            .or_default()
            .insert(session_id, new_handle);

        session_id
    }

    pub fn unregister_session_if_current(&self, identity: &ConnectionIdentity, session_id: u64) {
        if let Some(sessions_for_guid) = self.guid_sessions.get(&identity.guid) {
            sessions_for_guid.remove(&session_id);
            let should_drop_guid = sessions_for_guid.is_empty();
            drop(sessions_for_guid);
            if should_drop_guid {
                self.guid_sessions.remove(&identity.guid);
            }
        }
    }

    pub fn route_packet(&self, packet: RelayPacket) -> std::result::Result<(), RouteError> {
        let recipient = packet.recipient.trim();
        let Some(sessions_for_guid) = self.guid_sessions.get(recipient) else {
            return Err(RouteError);
        };
        let targets: Vec<SessionHandle> = sessions_for_guid
            .iter()
            .map(|entry| entry.value().clone())
            .collect();
        drop(sessions_for_guid);
        if targets.is_empty() {
            return Err(RouteError);
        }
        let mut sent = 0_usize;
        let mut stale_session_ids = Vec::new();
        for handle in targets {
            match handle.tx.send(OutboundCommand::Packet(packet.clone())) {
                Ok(()) => sent += 1,
                Err(_) => stale_session_ids.push(handle.session_id),
            }
        }

        if !stale_session_ids.is_empty() {
            if let Some(sessions_for_guid) = self.guid_sessions.get(recipient) {
                for session_id in stale_session_ids {
                    sessions_for_guid.remove(&session_id);
                }
                let should_drop_guid = sessions_for_guid.is_empty();
                drop(sessions_for_guid);
                if should_drop_guid {
                    self.guid_sessions.remove(recipient);
                }
            }
        }

        if sent == 0 {
            return Err(RouteError);
        }

        Ok(())
    }
}
