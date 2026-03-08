use std::cell::RefCell;

use crate::Contact;

#[derive(Clone, Debug)]
pub struct Call {
    parties: RefCell<Vec<CallParty>>,
}

impl Call {
    pub fn new(parties: Vec<Contact>) -> Self {
        let parties = parties
            .into_iter()
            .map(|p| p.into())
            .collect::<Vec<CallParty>>();
        let parties = RefCell::new(parties);
        Self { parties }
    }

    pub fn get_status(&self, public_key: &str) -> Option<CallPartyStatus> {
        for party in self.parties.borrow().iter() {
            if &party.contact.public_key == public_key {
                return Some(party.status);
            }
        }
        None
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

    pub fn stop(&self) {
        // ToDo: inform other parties
    }

    pub fn step(&self) {}
}

#[derive(Clone, Debug)]
pub struct CallParty {
    pub status: CallPartyStatus,
    pub contact: Contact,
}

#[derive(Clone, Copy, Debug)]
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
