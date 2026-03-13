use std::{
    cell::RefCell,
    collections::{HashMap, HashSet},
    path::PathBuf,
};

use rand_core::{OsRng, RngCore};

use anyhow::{Result, anyhow};

use crate::{
    AudioInterface, Call, ChatMessage, Contact, Database, HoshiNetClient,
    call::{CallParty, CallPartyStatus},
    database::DBReply,
    hoshi_net_client::{HoshiMessage, HoshiPayload},
};

pub struct HoshiClient {
    pub(crate) net: HoshiNetClient,
    db: Database,
    public_key: RefCell<String>,

    pub(crate) audio_interface: RefCell<Option<Box<dyn AudioInterface>>>,

    calls: RefCell<Vec<Call>>,
    calls_watchers: RefCell<Vec<Box<dyn Fn(&Self, &Vec<Call>)>>>,

    contacts: RefCell<HashMap<String, Contact>>,
    contacts_watchers: RefCell<Vec<Box<dyn Fn(&Self, &HashMap<String, Contact>)>>>,

    messages: RefCell<HashMap<String, HashMap<String, ChatMessage>>>,
    messages_watchers: RefCell<
        Vec<(
            String,
            Box<dyn Fn(&Self, &str, &HashMap<String, ChatMessage>)>,
        )>,
    >,
}

impl HoshiClient {
    pub fn new(db_path: Option<PathBuf>) -> Result<Self> {
        let net = HoshiNetClient::new();
        let path = db_path.unwrap_or_else(|| {
            let p = dirs::home_dir().unwrap().join(".hoshi");
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
        let incoming_calls = RefCell::new(vec![]);
        let incoming_call_watchers = RefCell::new(vec![]);

        let relay_list = if cfg!(debug_assertions) {
            vec!["ws://127.0.0.1:2800/".into()]
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

            audio_interface: RefCell::new(None),

            calls: incoming_calls,
            calls_watchers: incoming_call_watchers,

            contacts,
            contacts_watchers,

            messages,
            messages_watchers,
        })
    }

    pub fn set_audio_interface(&self, interface: Option<Box<dyn AudioInterface>>) {
        *self.audio_interface.borrow_mut() = interface;
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
        let mut call = Call::new(parties);

        let mut party: CallParty = self.own_contact().into();
        party.status = CallPartyStatus::Active;
        call.add_party(party);

        if let Some(interface) = self.audio_interface.borrow().as_ref() {
            if let Ok(stream) = interface.create(self, &call) {
                call.set_audio(Some(stream));
            }
        }

        self.calls.borrow_mut().push(call);

        self.calls_changed();
    }

    pub fn active_call_local_voice_activity(&self) -> f32 {
        0.0
    }

    pub fn active_call_voice_activity(&self, _public_key: &str) -> f32 {
        0.0
    }

    fn calls_changed(&self) {
        let calls = self.calls.borrow().clone();
        for f in self.calls_watchers.borrow().iter() {
            f(self, &calls);
        }
    }

    pub fn calls_watch<F>(&self, f: F)
    where
        F: Fn(&Self, &Vec<Call>) + 'static,
    {
        f(self, &self.calls.borrow());
        self.calls_watchers.borrow_mut().push(Box::new(f));
    }

    pub fn calls_push(&self, call: Call) {
        self.calls.borrow_mut().push(call);
        self.calls_changed();
    }

    pub fn call_get(&self, call_id: &str) -> Option<Call> {
        for call in self.calls.borrow().iter() {
            if call.id() == call_id {
                return Some(call.clone());
            }
        }
        None
    }

    pub fn calls(&self) -> Vec<Call> {
        self.calls.borrow().clone()
    }

    pub fn call_set_status(
        &self,
        call_id: &str,
        contact: Contact,
        status: CallPartyStatus,
    ) -> Result<()> {
        let mut found = false;
        for call in self.calls.borrow_mut().iter_mut() {
            if call.id() != call_id {
                continue;
            }

            let public_key = contact.public_key.clone();
            call.add_party(contact.clone().into());
            call.set_party_status(&public_key, status);
            for key in call.get_party_public_keys() {
                self.net.send(HoshiMessage::new(
                    self.public_key(),
                    key,
                    HoshiPayload::UpdateCallStatus {
                        call_id: call_id.to_string(),
                        party_id: public_key.to_string(),
                        status: status,
                    },
                ));
            }

            found = true;
            break;
        }
        if found {
            self.calls_changed();
            Ok(())
        } else {
            Err(anyhow!("Call {} not found", call_id))
        }
    }

    pub fn call_accept(&self, call_id: &str) -> Result<()> {
        self.call_set_status(call_id, self.own_contact(), CallPartyStatus::Active)
    }

    pub fn call_decline(&self, call_id: &str) -> Result<()> {
        self.call_set_status(call_id, self.own_contact(), CallPartyStatus::HungUp)
    }

    pub fn public_key(&self) -> String {
        self.public_key.borrow().clone()
    }

    pub fn own_contact(&self) -> Contact {
        Contact::new(self.public_key(), None)
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
            watcher(self, &contacts);
        }
    }

    fn messages_changed(&self, chat_id: String) {
        let watchers = self.messages_watchers.borrow();
        for (filter, watcher) in &*watchers {
            if filter.is_empty() || filter == &chat_id {
                let messages = self.messages.borrow();
                let messages = messages.get(&chat_id);
                if let Some(messages) = messages {
                    watcher(self, &chat_id, messages);
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
                HoshiPayload::InviteToCall { call_id } => {
                    let mut found = false;
                    for call in self.calls.borrow_mut().iter_mut() {
                        if call.id() == &call_id {
                            call.update_last_ring();
                            found = true;
                            break;
                        }
                    }
                    if !found {
                        let contact = self
                            .contact_get(&net_msg.from_key)
                            .unwrap_or_else(|| Contact::new(net_msg.from_key.clone(), None));
                        let call = Call::from_invite(call_id, contact, self.own_contact());
                        if let Some(interface) = self.audio_interface.borrow().as_ref() {
                            if let Ok(stream) = interface.create(self, &call) {
                                call.set_audio(Some(stream));
                            }
                        }

                        self.calls.borrow_mut().push(call);
                        self.calls_changed();
                    }
                }
                HoshiPayload::UpdateCallStatus {
                    call_id,
                    party_id,
                    status,
                } => {
                    for call in self.calls.borrow_mut().iter_mut() {
                        if call.id() == &call_id {
                            call.set_party_status(&party_id, status);
                            println!("{:?}", call);
                            break;
                        }
                    }
                }
                HoshiPayload::AudioChunk { call_id, chunk } => {
                    for call in self.calls.borrow_mut().iter_mut() {
                        if call.id() == &call_id {
                            let own_status = call.get_status(&self.public_key());
                            if matches!(own_status, Some(CallPartyStatus::Active)) {
                                call.receive_audio(chunk, &net_msg.from_key);
                            }
                            break;
                        }
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

        let before = self.calls.borrow().len();
        self.calls.borrow_mut().retain_mut(|call| {
            msgs += 1;
            call.step(self);
            call.active_or_ringing_party_count() > 1
        });
        if self.calls.borrow().len() != before {
            self.calls_changed();
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
        F: Fn(&Self, &str, &HashMap<String, ChatMessage>) + 'static,
    {
        {
            let chats = self.messages.borrow();
            if filter.is_empty() {
                for chat_id in chats.keys() {
                    if let Some(chat) = chats.get(chat_id) {
                        f(self, chat_id, chat);
                    }
                }
            } else {
                if let Some(chat) = chats.get(&filter) {
                    f(self, &filter, chat);
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
        F: Fn(&Self, &HashMap<String, Contact>) + 'static,
    {
        let contacts = self.contacts.borrow();
        f(self, &contacts);
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
