use std::cell::RefCell;

use serde::{Deserialize, Serialize};

use crate::{
    Contact,
    hoshi_client::HoshiClient,
    hoshi_net_client::{HoshiMessage, HoshiPayload},
};

#[derive(Clone, Debug)]
pub struct Call {
    id: String,
    parties: RefCell<Vec<CallParty>>,
    last_invite: RefCell<Option<std::time::Instant>>,
    pub call_started: RefCell<Option<std::time::Instant>>,
    pub call_ended: RefCell<Option<std::time::Instant>>,
    last_ring: RefCell<Option<std::time::Instant>>,
}

impl Call {
    pub fn new(parties: Vec<Contact>) -> Self {
        let parties = parties
            .into_iter()
            .map(|p| p.into())
            .collect::<Vec<CallParty>>();
        let parties = RefCell::new(parties);
        let id = uuid::Uuid::now_v7().to_string();
        Self {
            id,
            parties,
            last_invite: RefCell::new(None),
            call_started: RefCell::new(Some(std::time::Instant::now())),
            call_ended: RefCell::new(None),
            last_ring: RefCell::new(None),
        }
    }

    pub fn from_invite(id: String, caller: Contact) -> Self {
        Self {
            id,
            parties: RefCell::new(vec![caller.into()]),
            last_invite: RefCell::new(None),
            call_started: RefCell::new(Some(std::time::Instant::now())),
            call_ended: RefCell::new(None),
            last_ring: RefCell::new(Some(std::time::Instant::now())),
        }
    }

    pub fn update_last_ring(&self) {
        *self.last_ring.borrow_mut() = Some(std::time::Instant::now());
    }

    pub fn is_ring_timed_out(&self) -> bool {
        self.last_ring
            .borrow()
            .map_or(false, |t| t.elapsed().as_secs() >= 3)
    }

    pub fn should_auto_close(&self) -> bool {
        self.call_ended
            .borrow()
            .map_or(false, |t| t.elapsed().as_secs() >= 5)
    }

    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn get_status(&self, public_key: &str) -> Option<CallPartyStatus> {
        for party in self.parties.borrow().iter() {
            if &party.contact.public_key == public_key {
                return Some(party.status);
            }
        }
        None
    }

    pub fn set_party_status(&self, public_key: &str, status: CallPartyStatus) {
        for party in self.parties.borrow_mut().iter_mut() {
            if party.contact.public_key == public_key {
                party.status = status;
                return;
            }
        }
    }

    pub fn add_party(&self, contact: Contact) -> bool {
        if self.get_status(&contact.public_key).is_some() {
            return false;
        }
        self.parties.borrow_mut().push(contact.into());
        true
    }

    pub fn get_party_names(&self) -> Vec<String> {
        self.parties
            .borrow()
            .iter()
            .map(|p| p.contact.alias.clone())
            .collect()
    }

    pub fn get_party_public_keys(&self) -> Vec<String> {
        self.parties
            .borrow()
            .iter()
            .map(|p| p.contact.public_key.clone())
            .collect()
    }

    pub fn get_party_status_pairs(&self) -> Vec<(String, CallPartyStatus)> {
        self.parties
            .borrow()
            .iter()
            .map(|p| (p.contact.alias.clone(), p.status))
            .collect()
    }

    pub fn stop(&self) {
        // ToDo: inform other parties
    }

    pub fn step(&self, client: &HoshiClient) {
        let now = std::time::Instant::now();
        let should_send = self
            .last_invite
            .borrow()
            .map_or(true, |t| now.duration_since(t).as_secs() >= 1);

        if !should_send {
            return;
        }
        *self.last_invite.borrow_mut() = Some(now);

        for party in self.parties.borrow().iter() {
            if matches!(party.status, CallPartyStatus::Ringing) {
                client.net.send(HoshiMessage::new(
                    client.public_key(),
                    party.contact.public_key.clone(),
                    HoshiPayload::InviteToCall {
                        from_key: client.public_key(),
                        id: self.id.clone(),
                    },
                ));
            }
        }
    }
}

#[derive(Clone, Debug)]
pub struct CallParty {
    pub status: CallPartyStatus,
    pub contact: Contact,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub enum CallPartyStatus {
    Ringing,
    HungUp,
    Active,
}

impl From<Contact> for CallParty {
    fn from(value: Contact) -> Self {
        Self {
            contact: value,
            status: CallPartyStatus::Ringing,
        }
    }
}
