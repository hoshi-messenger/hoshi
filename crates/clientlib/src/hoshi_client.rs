use std::{cell::RefCell, collections::HashMap};

use anyhow::Result;

use crate::{Contact, Database, HoshiNetClient, database::DBReply};

pub struct HoshiClient {
    pub net: HoshiNetClient,

    db: Database,

    contacts: RefCell<HashMap<String, Contact>>,
    contacts_watchers: RefCell<Vec<Box<dyn Fn(&HashMap<String, Contact>)>>>,
}

impl HoshiClient {
    pub fn new() -> Result<Self> {
        let net = HoshiNetClient::new();
        let path = dirs::home_dir().unwrap().join(".hoshi");
        std::fs::create_dir_all(&path)?;
        let path = path.join("client.sqlite3");
        let db = Database::new(path)?;
        db.ping()?;
        db.contacts_get()?;

        let contacts = RefCell::new(HashMap::new());
        let contacts_watchers = RefCell::new(vec![]);

        Ok(Self {
            net,
            db,
            contacts,
            contacts_watchers,
        })
    }

    fn contacts_changed(&self) {
        let watchers = self.contacts_watchers.borrow();
        for watcher in &*watchers {
            let contacts = self.contacts.borrow();
            watcher(&contacts);
        }
    }

    fn handle_db_msg(&self, msg: DBReply) {
        match msg {
            DBReply::Pong => {
                println!("Client/DB: Pong");
            }
            DBReply::Shutdown => {
                println!("Client/DB: Shutdown");
            }
            DBReply::Contacts(new_contacts) => {
                {
                    let mut contacts = self.contacts.borrow_mut();
                    contacts.clear();

                    for c in new_contacts {
                        let public_key = c.public_key.clone();
                        contacts.insert(public_key, c);
                    }
                }
                self.contacts_changed();
            }
        }
    }

    /// Main function a client MUST call regularly for the clientlib
    /// to work as expected. It communicates with the various other
    /// threads and updates the internal state as well as fires callbacks.
    ///
    /// The exact interval isn't that important, the GTK client calls it
    /// every 8ms/64ms depending on whether the last call actually handled
    /// any messages.
    pub fn step(&self) -> u32 {
        let mut msgs = 0;

        // Only handle at most 32 msgs per iteration, make sure the
        // calling event loop doesn't block for too long, exact value
        // will have to be fine-tuned once we have an actual workload
        for _i in 1..32 {
            if let Some(msg) = self.db.recv() {
                msgs += 1;
                self.handle_db_msg(msg);
            } else {
                break;
            }
        }

        msgs
    }

    pub fn with_contacts<F>(&self, f: F)
    where
        F: FnOnce(&HashMap<String, Contact>) + 'static,
    {
        let contacts = self.contacts.borrow();
        f(&contacts);
    }

    pub fn contacts_watch<F>(&self, f: F)
    where
        F: Fn(&HashMap<String, Contact>) + 'static,
    {
        let mut watchers = self.contacts_watchers.borrow_mut();
        watchers.push(Box::new(f));
    }

    pub fn contact_get(&self, public_key: &str) -> Option<Contact> {
        self.contacts.borrow().get(public_key).map(|c| c.clone())
    }

    pub fn contact_upsert(&self, contact: Contact) -> Result<()> {
        {
            let mut contacts = self.contacts.borrow_mut();
            let contact = contact.clone();
            contacts.insert(contact.public_key.clone(), contact);
        }
        self.db.contact_upsert(contact)?;
        self.contacts_changed();

        Ok(())
    }

    pub fn contact_delete(&self, public_key: &str) -> Result<()> {
        {
            let mut contacts = self.contacts.borrow_mut();
            contacts.remove(public_key);
        }
        self.db.contact_delete(public_key.to_string())?;
        self.contacts_changed();

        Ok(())
    }
}

impl std::fmt::Debug for HoshiClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HoshiClient")
            .field("net", &self.net)
            .field("db", &self.db)
            .field("contacts", &self.contacts)
            .field(
                "contacts_watchers",
                &format!("[{} watchers]", self.contacts_watchers.borrow().len()),
            )
            .finish()
    }
}
