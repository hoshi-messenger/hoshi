use std::{
    cell::RefCell,
    collections::{HashMap, HashSet},
    path::PathBuf,
};

use rand_core::{OsRng, RngCore};

use anyhow::Result;

use crate::{
    Call, ChatMessage, Contact, Database, HoshiNetClient,
    call::CallPartyStatus,
    database::DBReply,
    hoshi_net_client::{HoshiMessage, HoshiPayload},
};

pub struct HoshiClient {
    pub(crate) net: HoshiNetClient,
    db: Database,
    public_key: RefCell<String>,

    active_call: RefCell<Option<Call>>,
    active_call_watchers: RefCell<Vec<Box<dyn Fn(&Option<Call>)>>>,

    incoming_calls: RefCell<Vec<Call>>,
    incoming_call_watchers: RefCell<Vec<Box<dyn Fn(&Vec<Call>)>>>,

    contacts: RefCell<HashMap<String, Contact>>,
    contacts_watchers: RefCell<Vec<Box<dyn Fn(&HashMap<String, Contact>)>>>,

    messages: RefCell<HashMap<String, HashMap<String, ChatMessage>>>,
    messages_watchers: RefCell<Vec<(String, Box<dyn Fn(&str, &HashMap<String, ChatMessage>)>)>>,
}

impl HoshiClient {
    pub fn new(db_path: Option<PathBuf>) -> Result<Self> {
        let net = HoshiNetClient::new();
        let path = db_path.unwrap_or_else(|| {
            let p = dirs::home_dir().unwrap().join(".hoshi");
            std::fs::create_dir_all(&p).unwrap();
            p.join("client.sqlite3")
        });
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let db = Database::new(path)?;

        let public_key = match db.config_get_blocking("public_key") {
            Some(bytes) => String::from_utf8(bytes).expect("public_key config is not valid UTF-8"),
            None => {
                let mut bytes = [0u8; 32];
                OsRng.fill_bytes(&mut bytes);
                let key: String = bytes.iter().map(|b| format!("{:02x}", b)).collect();
                db.config_set("public_key", key.as_bytes().to_vec())?;
                key
            }
        };

        db.messages_get()?;
        db.contacts_get()?;

        let contacts = RefCell::new(HashMap::new());
        let contacts_watchers = RefCell::new(vec![]);
        let messages = RefCell::new(HashMap::new());
        let messages_watchers = RefCell::new(vec![]);
        let active_call = RefCell::new(None);
        let active_call_watchers = RefCell::new(vec![]);
        let incoming_calls = RefCell::new(vec![]);
        let incoming_call_watchers = RefCell::new(vec![]);

        let relay_list = if cfg!(debug_assertions) {
            vec!["ws://127.0.0.1:2700/".into()]
        } else {
            vec!["wss://hoshi.rubhub.net/relay/asuka/".into()]
        };
        let relay_list = RefCell::new(relay_list);
        {
            let relay_list = relay_list.borrow();
            net.update_relays(&relay_list);
        }
        net.set_public_key(public_key.clone());

        Ok(Self {
            net,
            db,
            public_key: RefCell::new(public_key),
            contacts,
            contacts_watchers,
            active_call,
            active_call_watchers,
            incoming_calls,
            incoming_call_watchers,
            messages,
            messages_watchers,
        })
    }

    fn request_chat_messages(&self, contact: &Contact) {
        let msg = HoshiMessage::new(
            self.public_key(),
            contact.public_key.to_string(),
            HoshiPayload::RequestChatMessages,
        );
        self.net.send(msg);
    }

    fn send_chat_history(&self, public_key: &str) {
        let chat_id = ChatMessage::calc_chat_id(public_key, &self.public_key());
        if let Some(chat) = self.messages.borrow().get(&chat_id) {
            for msg in chat.values() {
                self.net.send(HoshiMessage::new(
                    self.public_key(),
                    public_key.to_string(),
                    HoshiPayload::ChatMessage(msg.clone()),
                ));
            }
        }
    }

    pub fn call_start(&self, parties: Vec<Contact>) {
        println!("Call Start: {:?}", parties);
        if self.active_call.borrow().is_some() {
            return;
        }
        let call = Call::new(parties);
        self.active_call.replace(Some(call));
        self.active_call_changed();
    }

    pub fn call_stop(&self) {
        {
            let mut active = self.active_call.borrow_mut();
            let call = active.take();
            if let Some(call) = call {
                for key in call.get_party_public_keys() {
                    self.net.send(HoshiMessage::new(
                        self.public_key(),
                        key,
                        HoshiPayload::UpdateCallStatus {
                            id: call.id().to_string(),
                            status: CallPartyStatus::HungUp,
                        },
                    ));
                }
                call.stop();
            }
        }
        self.active_call_changed();
    }

    fn active_call_changed(&self) {
        let call = self.active_call.borrow().clone();
        for f in self.active_call_watchers.borrow().iter() {
            f(&call);
        }
    }

    pub fn active_call_local_voice_activity(&self) -> f32 {
        self.active_call
            .borrow()
            .as_ref()
            .map(|call| call.get_local_voice_activity())
            .unwrap_or(0.0)
    }

    pub fn active_call_voice_activity(&self, public_key: &str) -> f32 {
        self.active_call
            .borrow()
            .as_ref()
            .map(|call| call.get_voice_activity(public_key))
            .unwrap_or(0.0)
    }

    pub fn active_call_watch<F>(&self, f: F)
    where
        F: Fn(&Option<Call>) + 'static,
    {
        f(&self.active_call.borrow());
        self.active_call_watchers.borrow_mut().push(Box::new(f));
    }

    fn incoming_calls_changed(&self) {
        let calls = self.incoming_calls.borrow().clone();
        for f in self.incoming_call_watchers.borrow().iter() {
            f(&calls);
        }
    }

    pub fn incoming_calls_watch<F>(&self, f: F)
    where
        F: Fn(&Vec<Call>) + 'static,
    {
        f(&self.incoming_calls.borrow());
        self.incoming_call_watchers.borrow_mut().push(Box::new(f));
    }

    pub fn incoming_call_push(&self, call: Call) {
        self.incoming_calls.borrow_mut().push(call);
        self.incoming_calls_changed();
    }

    pub fn incoming_call_accept(&self, call_id: &str) {
        let call = {
            let mut calls = self.incoming_calls.borrow_mut();
            let pos = calls.iter().position(|c| c.id() == call_id);
            pos.map(|i| calls.remove(i))
        };
        if let Some(call) = call {
            for key in call.get_party_public_keys() {
                self.net.send(HoshiMessage::new(
                    self.public_key(),
                    key,
                    HoshiPayload::UpdateCallStatus {
                        id: call_id.to_string(),
                        status: CallPartyStatus::Active,
                    },
                ));
            }
            self.active_call.replace(Some(call));
            self.incoming_calls_changed();
            self.active_call_changed();
        }
    }

    pub fn incoming_call_decline(&self, call_id: &str) {
        let call = {
            let mut calls = self.incoming_calls.borrow_mut();
            let pos = calls.iter().position(|c| c.id() == call_id);
            pos.map(|i| calls.remove(i))
        };
        if let Some(call) = call {
            for key in call.get_party_public_keys() {
                self.net.send(HoshiMessage::new(
                    self.public_key(),
                    key,
                    HoshiPayload::UpdateCallStatus {
                        id: call_id.to_string(),
                        status: CallPartyStatus::HungUp,
                    },
                ));
            }
            self.incoming_calls_changed();
        }
    }

    pub fn public_key(&self) -> String {
        self.public_key.borrow().clone()
    }

    pub fn set_public_key(&self, key: String) -> Result<()> {
        self.net.set_public_key(key.clone());
        self.net.disconnect_all();
        self.db.config_set("public_key", key.as_bytes().to_vec())?;
        *self.public_key.borrow_mut() = key;
        Ok(())
    }

    fn contacts_changed(&self) {
        let watchers = self.contacts_watchers.borrow();
        for watcher in &*watchers {
            let contacts = self.contacts.borrow();
            watcher(&contacts);
        }
    }

    fn messages_changed(&self, chat_id: String) {
        let watchers = self.messages_watchers.borrow();
        for (filter, watcher) in &*watchers {
            if filter.is_empty() || filter == &chat_id {
                let messages = self.messages.borrow();
                let messages = messages.get(&chat_id);
                if let Some(messages) = messages {
                    watcher(&chat_id, messages);
                }
            }
        }
    }

    fn handle_db_msg(&self, msg: DBReply) {
        match msg {
            DBReply::Shutdown => {
                panic!("Client/DB: Shutdown - we should never receive this!");
            }
            DBReply::Config(_) => {
                // Config replies are consumed synchronously at startup via config_get_blocking;
                // receiving one here would be a bug.
                panic!("Client/DB: unexpected Config reply in step()");
            }
            DBReply::Contacts(new_contacts) => {
                {
                    self.contacts.borrow_mut().clear();

                    for c in new_contacts {
                        if self.contact_get(&c.public_key).is_none() {
                            self.request_chat_messages(&c);
                            self.send_chat_history(&c.public_key);
                        }
                        let public_key = c.public_key.clone();
                        self.contacts.borrow_mut().insert(public_key, c);
                    }
                }
                self.contacts_changed();
            }
            DBReply::Messages(msgs) => {
                let mut chat_ids = HashSet::new();
                for msg in msgs {
                    let chat_id = msg.chat_id();
                    chat_ids.insert(chat_id.to_string());
                    self.save_chat_message(msg);
                }

                for chat_id in chat_ids.drain() {
                    self.messages_changed(chat_id);
                }
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
        for net_msg in self.net.step() {
            match net_msg.payload {
                HoshiPayload::ChatMessage(chat_msg) => {
                    let chat_id = chat_msg.chat_id();
                    if self.save_chat_message(chat_msg.clone()) {
                        let _ = self.db.message_upsert(chat_msg);
                        self.messages_changed(chat_id);
                    }
                }
                HoshiPayload::RequestChatMessages => {
                    self.send_chat_history(&net_msg.from_key);
                }
                HoshiPayload::InviteToCall { from_key, id } => {
                    let active_call_id = self
                        .active_call
                        .borrow()
                        .as_ref()
                        .map(|c| c.id().to_string());
                    match active_call_id {
                        Some(ref our_id) if our_id == &id => {
                            // Repeat invite for our current call — confirm Active back
                            if let Some(call) = self.active_call.borrow().as_ref() {
                                call.set_party_status(&from_key, CallPartyStatus::Active);
                                self.net.send(HoshiMessage::new(
                                    self.public_key(),
                                    from_key,
                                    HoshiPayload::UpdateCallStatus {
                                        id: id.to_string(),
                                        status: CallPartyStatus::Active,
                                    },
                                ));
                            }
                            self.active_call_changed();
                        }
                        Some(ref our_id) => {
                            let in_our_call = self
                                .active_call
                                .borrow()
                                .as_ref()
                                .map_or(false, |c| c.get_status(&from_key).is_some());
                            if in_our_call {
                                // Simultaneous call: lower ID wins
                                let (winner_id, loser_id) = if id < *our_id {
                                    (id.clone(), our_id.clone())
                                } else {
                                    (our_id.clone(), id.clone())
                                };
                                self.net.send(HoshiMessage::new(
                                    self.public_key(),
                                    from_key.clone(),
                                    HoshiPayload::UpdateCallStatus {
                                        id: winner_id.clone(),
                                        status: CallPartyStatus::Active,
                                    },
                                ));
                                self.net.send(HoshiMessage::new(
                                    self.public_key(),
                                    from_key.clone(),
                                    HoshiPayload::UpdateCallStatus {
                                        id: loser_id.clone(),
                                        status: CallPartyStatus::HungUp,
                                    },
                                ));
                                if id < *our_id {
                                    // Incoming wins — hang up ours, accept theirs
                                    self.active_call.borrow_mut().take();
                                    let contact = self
                                        .contact_get(&from_key)
                                        .unwrap_or_else(|| Contact::new(from_key.clone(), None));
                                    let call = Call::from_invite(id.clone(), contact);
                                    call.set_party_status(&from_key, CallPartyStatus::Active);
                                    self.active_call.replace(Some(call));
                                    self.active_call_changed();
                                } else {
                                    // Ours wins — remove incoming if it snuck into the queue
                                    let pos = self
                                        .incoming_calls
                                        .borrow()
                                        .iter()
                                        .position(|c| c.id() == id);
                                    if let Some(pos) = pos {
                                        self.incoming_calls.borrow_mut().remove(pos);
                                        self.incoming_calls_changed();
                                    }
                                }
                            } else {
                                // Different party calling while we're in a call
                                let existing = self
                                    .incoming_calls
                                    .borrow()
                                    .iter()
                                    .position(|c| c.id() == id);
                                match existing {
                                    Some(i) => {
                                        self.incoming_calls.borrow()[i].update_last_ring();
                                    }
                                    None => {
                                        let contact =
                                            self.contact_get(&from_key).unwrap_or_else(|| {
                                                Contact::new(from_key.clone(), None)
                                            });
                                        self.incoming_calls
                                            .borrow_mut()
                                            .push(Call::from_invite(id, contact));
                                        self.incoming_calls_changed();
                                    }
                                }
                            }
                        }
                        None => {
                            // No active call — add to incoming or refresh last_ring if already there
                            let existing = self
                                .incoming_calls
                                .borrow()
                                .iter()
                                .position(|c| c.id() == id);
                            match existing {
                                Some(i) => {
                                    self.incoming_calls.borrow()[i].update_last_ring();
                                }
                                None => {
                                    let contact = self
                                        .contact_get(&from_key)
                                        .unwrap_or_else(|| Contact::new(from_key.clone(), None));
                                    self.incoming_calls
                                        .borrow_mut()
                                        .push(Call::from_invite(id, contact));
                                    self.incoming_calls_changed();
                                }
                            }
                        }
                    }
                }
                HoshiPayload::UpdateCallStatus { id, status } => {
                    if let Some(call) = self.active_call.borrow().as_ref() {
                        if call.id() == id {
                            call.set_party_status(&net_msg.from_key, status);
                            if matches!(status, CallPartyStatus::HungUp) {
                                *call.call_ended.borrow_mut() = Some(std::time::Instant::now());
                            }
                        }
                    }
                    if matches!(status, CallPartyStatus::HungUp) {
                        let pos = self
                            .incoming_calls
                            .borrow()
                            .iter()
                            .position(|c| c.id() == id);
                        if let Some(pos) = pos {
                            self.incoming_calls.borrow_mut().remove(pos);
                            self.incoming_calls_changed();
                        }
                    }
                    self.active_call_changed();
                }
                HoshiPayload::AudioChunk(chunk) => {
                    let valid = self.active_call.borrow().as_ref().map_or(false, |call| {
                        call.id() == chunk.id() && call.get_status(&net_msg.from_key).is_some()
                    });
                    if valid {
                        if let Some(call) = self.active_call.borrow().as_ref() {
                            call.receive_audio(chunk, &net_msg.from_key);
                        }
                    } else {
                        eprintln!(
                            "AudioChunk dropped: call_id={} from={}",
                            chunk.id(),
                            net_msg.from_key
                        );
                    }
                }
                _ => {}
            }
        }

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

        if let Some(call) = self.active_call.borrow().as_ref() {
            call.step(self);
            // This way the we signal to the client that we should
            // run the step function more frequently while a call is active
            msgs += 1;
        }

        if self
            .active_call
            .borrow()
            .as_ref()
            .map_or(false, |c| c.should_auto_close())
        {
            self.active_call.borrow_mut().take();
            self.active_call_changed();
        }

        let before = self.incoming_calls.borrow().len();
        self.incoming_calls
            .borrow_mut()
            .retain(|c| !c.is_ring_timed_out());
        if self.incoming_calls.borrow().len() != before {
            self.incoming_calls_changed();
        }

        msgs
    }

    fn save_chat_message(&self, msg: ChatMessage) -> bool {
        let chat_id = msg.chat_id();
        {
            let mut chats = self.messages.borrow_mut();
            let chat = chats.get_mut(&chat_id);
            if let Some(chat) = chat {
                // Skip duplicate messages
                if chat.contains_key(&msg.id) {
                    return false;
                }
                chat.insert(msg.id.to_string(), msg.clone());
                true
            } else {
                let mut chat = HashMap::new();
                chat.insert(msg.id.to_string(), msg.clone());
                chats.insert(chat_id.clone(), chat);
                true
            }
        }
    }

    /// Update or insert a message, prefer insertion since in the future this
    /// function might be removed and replaced with a message_insert function,
    /// meant as a simple way to get an MVP working.
    pub fn message_upsert(&self, msg: ChatMessage) -> Result<()> {
        let chat_id = msg.chat_id();
        self.save_chat_message(msg.clone());
        self.db.message_upsert(msg.clone())?;
        self.net.send(HoshiMessage::new(
            self.public_key(),
            msg.to.clone(),
            HoshiPayload::ChatMessage(msg),
        ));
        self.messages_changed(chat_id);

        Ok(())
    }

    /// Call this function to get notified whenever messages in a
    /// particular chat_id as specified by filter changes, can also
    /// be left empty to get notified about all messages.
    /// Your callback f gets called immediately on registering with a
    /// current snapshot and then additionally whenever a message changes.
    #[inline]
    pub fn messages_watch<F>(&self, filter: String, f: F)
    where
        F: Fn(&str, &HashMap<String, ChatMessage>) + 'static,
    {
        {
            let chats = self.messages.borrow();
            if filter.is_empty() {
                for chat_id in chats.keys() {
                    if let Some(chat) = chats.get(chat_id) {
                        f(chat_id, chat);
                    }
                }
            } else {
                if let Some(chat) = chats.get(&filter) {
                    f(&filter, chat);
                }
            }
        }

        let mut watchers = self.messages_watchers.borrow_mut();
        watchers.push((filter.to_string(), Box::new(f)));
    }

    /// Call f with a current snapshot of the current contacts once
    #[inline]
    pub fn with_contacts<F>(&self, f: F)
    where
        F: FnOnce(&HashMap<String, Contact>) + 'static,
    {
        let contacts = self.contacts.borrow();
        f(&contacts);
    }

    /// Use this function so that f gets called whenever a contact changes.
    /// Also gets called once immediately with a current snapshot of the local
    /// state.
    #[inline]
    pub fn contacts_watch<F>(&self, f: F)
    where
        F: Fn(&HashMap<String, Contact>) + 'static,
    {
        let contacts = self.contacts.borrow();
        f(&contacts);
        let mut watchers = self.contacts_watchers.borrow_mut();
        watchers.push(Box::new(f));
    }

    /// Lookup a particular public_key in the current snapshot of Contancts
    #[inline]
    pub fn contact_get(&self, public_key: &str) -> Option<Contact> {
        self.contacts.borrow().get(public_key).map(|c| c.clone())
    }

    /// Update or Insert a particular Contact, currently only persists in the
    /// local DB but in the future should also propagate to other devices on the
    /// network if they share the same user.
    pub fn contact_upsert(&self, contact: Contact) -> Result<()> {
        {
            let mut contacts = self.contacts.borrow_mut();
            let contact = contact.clone();
            contacts.insert(contact.public_key.clone(), contact);
        }
        self.request_chat_messages(&contact);
        self.db.contact_upsert(contact)?;
        self.contacts_changed();

        Ok(())
    }

    /// Remove a contact, for now only removes it from the local DB but should
    /// propagate over the network to other devices sharing the same user in
    /// the future.
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
            .field("db", &self.db)
            .field("contacts", &self.contacts)
            .field(
                "contacts_watchers",
                &format!("[{} watchers]", self.contacts_watchers.borrow().len()),
            )
            .finish()
    }
}
