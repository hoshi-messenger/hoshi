use hoshi_protocol::relay::RelayPacket;
use std::{collections::HashSet, sync::atomic::Ordering};

use tokio::sync::mpsc;

use super::ServerState;

#[derive(Debug, Clone)]
pub struct ConnectionIdentity {
    pub client_guid: String,
    pub device_guid: String,
}

#[derive(Debug, Clone)]
pub enum OutboundCommand {
    Packet(RelayPacket),
    Close,
}

#[derive(Debug, Clone)]
pub(super) struct SessionHandle {
    pub(super) session_id: u64,
    pub(super) client_guid: String,
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

        let new_handle = SessionHandle {
            session_id,
            client_guid: identity.client_guid.clone(),
            tx: tx.clone(),
        };

        if let Some(old_handle) = self
            .device_sessions
            .insert(identity.device_guid.clone(), new_handle)
        {
            let _ = old_handle.tx.send(OutboundCommand::Close);
            self.remove_device_from_client_index(&identity.device_guid, &old_handle.client_guid);
        }

        let client_set = self
            .client_devices
            .entry(identity.client_guid.clone())
            .or_default();
        client_set.insert(identity.device_guid.clone());

        session_id
    }

    pub fn unregister_session_if_current(&self, identity: &ConnectionIdentity, session_id: u64) {
        let should_remove = self
            .device_sessions
            .get(&identity.device_guid)
            .map(|entry| entry.session_id == session_id)
            .unwrap_or(false);

        if !should_remove {
            return;
        }

        self.device_sessions.remove(&identity.device_guid);
        self.remove_device_from_client_index(&identity.device_guid, &identity.client_guid);
    }

    fn remove_device_from_client_index(&self, device_guid: &str, client_guid: &str) {
        let should_drop_client = if let Some(client_set) = self.client_devices.get(client_guid) {
            client_set.remove(device_guid);
            client_set.is_empty()
        } else {
            false
        };

        if should_drop_client {
            self.client_devices.remove(client_guid);
        }
    }

    pub fn route_packet(&self, packet: RelayPacket) -> std::result::Result<(), RouteError> {
        let recipient = packet.recipient.trim();

        let mut targets: HashSet<String> = HashSet::new();
        if self.device_sessions.contains_key(recipient) {
            targets.insert(recipient.to_string());
        }
        if let Some(client_devices) = self.client_devices.get(recipient) {
            for device_guid in client_devices.iter() {
                targets.insert(device_guid.to_string());
            }
        }

        if targets.is_empty() {
            return Err(RouteError);
        }

        let mut sent = 0_usize;
        let mut stale = Vec::new();

        for device_guid in targets {
            let Some(handle) = self
                .device_sessions
                .get(&device_guid)
                .map(|entry| entry.clone())
            else {
                continue;
            };

            match handle.tx.send(OutboundCommand::Packet(packet.clone())) {
                Ok(()) => sent += 1,
                Err(_) => stale.push((device_guid, handle.session_id, handle.client_guid)),
            }
        }

        for (device_guid, session_id, client_guid) in stale {
            let should_remove = self
                .device_sessions
                .get(&device_guid)
                .map(|entry| entry.session_id == session_id)
                .unwrap_or(false);
            if should_remove {
                self.device_sessions.remove(&device_guid);
                self.remove_device_from_client_index(&device_guid, &client_guid);
            }
        }

        if sent == 0 {
            return Err(RouteError);
        }

        Ok(())
    }
}
