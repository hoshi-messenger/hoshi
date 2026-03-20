use std::{
    cell::RefCell,
    collections::{HashMap, HashSet},
    path::PathBuf,
    time::{Duration, Instant},
};

use anyhow::{Result, anyhow};

use crate::{
    AudioInterface, Call, ChatMessage, Contact, HoshiNetClient, HoshiNode, HoshiNodePayload,
    NodeStore,
    call::{CallPartyEvent, CallPartyStatus},
    chat_path,
    hoshi_net_client::{HoshiMessage, HoshiPayload},
    identity::HoshiIdentity,
    user_path,
};

pub struct HoshiClient {
    pub(crate) net: HoshiNetClient,
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

    nodes: RefCell<NodeStore>,
    node_interests: RefCell<HashSet<String>>,

    last_sync: RefCell<Instant>,
    sync_index: RefCell<usize>,
}

impl HoshiClient {
    pub fn new(data_dir: Option<PathBuf>) -> Result<Self> {
        let data_dir = data_dir.unwrap_or_else(|| dirs::home_dir().unwrap().join(".hoshi"));
        std::fs::create_dir_all(&data_dir)?;
        let node_store_path = data_dir.join("nodes");

        let mut nodes = NodeStore::new(Some(node_store_path), String::new());
        let identity = match nodes.config_get("ed25519_seed") {
            Some(seed_hex) => {
                let seed_bytes: Vec<u8> = (0..seed_hex.len())
                    .step_by(2)
                    .map(|i| u8::from_str_radix(&seed_hex[i..i + 2], 16).unwrap())
                    .collect();
                let seed: [u8; 32] = seed_bytes
                    .try_into()
                    .expect("ed25519_seed must be 32 bytes");
                HoshiIdentity::from_seed(seed)
            }
            None => {
                let identity = HoshiIdentity::generate();
                let seed_hex: String = identity
                    .seed()
                    .iter()
                    .map(|b| format!("{:02x}", b))
                    .collect();
                nodes.config_set("ed25519_seed", &seed_hex);
                nodes.config_set("public_key", &identity.public_key_hex());
                identity
            }
        };
        let public_key = identity.public_key_hex();
        nodes.set_public_key(public_key.clone());

        let tls_config = identity.make_client_tls_config();
        let net = HoshiNetClient::new(tls_config);

        let loaded_contacts = nodes.contacts();
        let mut contacts = HashMap::new();
        for c in loaded_contacts {
            contacts.insert(c.public_key.clone(), c);
        }
        let contacts = RefCell::new(contacts);
        let contacts_watchers = RefCell::new(vec![]);
        let messages_watchers = RefCell::new(vec![]);
        let incoming_calls = RefCell::new(vec![]);
        let incoming_call_watchers = RefCell::new(vec![]);

        // Register node interests and load messages for all existing chats
        let mut node_interests = HashSet::new();
        let mut messages_map: HashMap<String, HashMap<String, ChatMessage>> = HashMap::new();
        for contact in contacts.borrow().values() {
            let cp = chat_path(&public_key, &contact.public_key);
            node_interests.insert(cp.clone());
            if contact.contact_type != crate::ContactType::Blocked {
                node_interests.insert(user_path(&contact.public_key));
            }
            let msgs =
                ChatMessage::messages_from_nodes(&mut nodes, &cp, &public_key, &contact.public_key);
            if !msgs.is_empty() {
                messages_map.insert(cp, msgs);
            }
        }

        let relay_list = if cfg!(debug_assertions) {
            vec!["wss://127.0.0.1:2800/".into()]
        } else {
            vec!["wss://hoshi.rubhub.net/relay/asuka/".into()]
        };
        let relay_list = RefCell::new(relay_list);
        {
            let relay_list = relay_list.borrow();
            net.update_relays(&relay_list);
        }
        Ok(Self {
            net,
            public_key: RefCell::new(public_key.clone()),

            audio_interface: RefCell::new(None),

            calls: incoming_calls,
            calls_watchers: incoming_call_watchers,

            contacts,
            contacts_watchers,

            messages: RefCell::new(messages_map),
            messages_watchers,

            nodes: RefCell::new(nodes),
            node_interests: RefCell::new(node_interests),

            last_sync: RefCell::new(Instant::now()),
            sync_index: RefCell::new(0),
        })
    }

    pub fn set_audio_interface(&self, interface: Option<Box<dyn AudioInterface>>) {
        *self.audio_interface.borrow_mut() = interface;
    }

    pub fn node_interest_add(&self, path: &str) {
        self.node_interests.borrow_mut().insert(path.to_string());
    }

    pub fn node_interest_remove(&self, path: &str) {
        self.node_interests.borrow_mut().remove(path);
    }

    fn is_node_interested(&self, path: &str) -> bool {
        let interests = self.node_interests.borrow();
        for interest in interests.iter() {
            if path == interest {
                return true;
            }
            if let Some(rest) = path.strip_prefix(interest.as_str()) {
                if rest.starts_with('/') {
                    return true;
                }
            }
        }
        // Accept chat messages from unknown senders if we're a valid participant
        if path.starts_with("/chat/") {
            return crate::node::peer_key_from_chat_path(&self.public_key(), path).is_some();
        }
        false
    }

    fn node_sync(&self, peer_key: &str, path: &str, have_local_data: bool) {
        if have_local_data {
            // We have data but hashes differ — drill down to find differences.
            // Ask the peer for their children so we can learn about their data:
            self.net.send(HoshiMessage::new(
                self.public_key(),
                peer_key.to_string(),
                HoshiPayload::NodeList {
                    path: path.to_string(),
                },
            ));
            // Also advertise our own children so the peer can learn about ours:
            let mut nodes = self.nodes.borrow_mut();
            let child_paths: Vec<String> = nodes
                .children(path)
                .iter()
                .map(|n| n.path.clone())
                .collect();
            for child_path in child_paths {
                let h = nodes.hash(&child_path);
                self.net.send(HoshiMessage::new(
                    self.public_key(),
                    peer_key.to_string(),
                    HoshiPayload::NodeAdvertise {
                        path: child_path,
                        hash: h.as_bytes().to_vec(),
                    },
                ));
            }
        } else {
            // We have nothing for this path — request the data directly
            self.net.send(HoshiMessage::new(
                self.public_key(),
                peer_key.to_string(),
                HoshiPayload::NodeRequest {
                    path: path.to_string(),
                },
            ));
        }
    }

    fn advertise_chat(&self, peer_key: &str) {
        let path = chat_path(&self.public_key(), peer_key);
        self.node_interest_add(&path);
        let mut nodes = self.nodes.borrow_mut();
        let h = nodes.hash(&path);
        self.net.send(HoshiMessage::new(
            self.public_key(),
            peer_key.to_string(),
            HoshiPayload::NodeAdvertise {
                path,
                hash: h.as_bytes().to_vec(),
            },
        ));
    }

    fn advertise_user(&self, peer_key: &str) {
        let path = user_path(&self.public_key());
        let mut nodes = self.nodes.borrow_mut();
        let h = nodes.hash(&path);
        self.net.send(HoshiMessage::new(
            self.public_key(),
            peer_key.to_string(),
            HoshiPayload::NodeAdvertise {
                path,
                hash: h.as_bytes().to_vec(),
            },
        ));
    }

    fn sync_peer_user(&self, peer_key: &str) {
        let path = user_path(peer_key);
        let mut nodes = self.nodes.borrow_mut();
        let have_local = nodes.get(&path).is_some();
        drop(nodes);
        self.node_sync(peer_key, &path, have_local);
    }

    fn rebuild_chat_messages(&self, cp: &str, peer_key: &str) {
        let msgs = {
            let mut nodes = self.nodes.borrow_mut();
            ChatMessage::messages_from_nodes(&mut nodes, cp, &self.public_key(), peer_key)
        };
        self.messages.borrow_mut().insert(cp.to_string(), msgs);
        self.messages_changed(cp.to_string());
    }

    /// Derive the peer key for a chat path from the XOR hash
    pub fn peer_key_for_chat_path(&self, cp: &str) -> Option<String> {
        crate::node::peer_key_from_chat_path(&self.public_key(), cp)
    }

    pub fn call_start(&self, parties: Vec<Contact>) {
        let call = Call::new(self.public_key(), parties);

        if let Some(interface) = self.audio_interface.borrow().as_ref() {
            if let Ok(stream) = interface.create(self, &call) {
                call.set_audio(Some(stream));
            }
        }

        self.send_call_state(&call);
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

    fn send_call_state(&self, call: &Call) {
        let my_key = self.public_key();
        for key in call.non_hungup_party_keys() {
            if key == my_key {
                continue;
            }
            self.net.send(HoshiMessage::new(
                my_key.clone(),
                key,
                HoshiPayload::UpdateCallState {
                    call_id: call.id().to_string(),
                    events: call.events().clone(),
                },
            ));
        }
    }

    pub fn call_accept(&self, call_id: &str) -> Result<()> {
        let msgs = {
            let mut calls = self.calls.borrow_mut();
            let call = calls
                .iter_mut()
                .find(|c| c.id() == call_id)
                .ok_or_else(|| anyhow!("Call {} not found", call_id))?;
            call.add_event(CallPartyEvent::new(
                self.public_key(),
                CallPartyStatus::Active,
            ));
            self.build_call_state_messages(call)
        };
        for msg in msgs {
            self.net.send(msg);
        }
        self.calls_changed();
        Ok(())
    }

    pub fn call_decline(&self, call_id: &str) -> Result<()> {
        let msgs = {
            let mut calls = self.calls.borrow_mut();
            let call = calls
                .iter_mut()
                .find(|c| c.id() == call_id)
                .ok_or_else(|| anyhow!("Call {} not found", call_id))?;
            call.add_event(CallPartyEvent::new(
                self.public_key(),
                CallPartyStatus::HungUp,
            ));
            self.build_call_state_messages(call)
        };
        for msg in msgs {
            self.net.send(msg);
        }
        self.calls_changed();
        Ok(())
    }

    pub fn call_invite_party(&self, call_id: &str, contact: Contact) -> Result<()> {
        let msgs = {
            let mut calls = self.calls.borrow_mut();
            let call = calls
                .iter_mut()
                .find(|c| c.id() == call_id)
                .ok_or_else(|| anyhow!("Call {} not found", call_id))?;
            call.add_event_with_contact(
                CallPartyEvent::new(contact.public_key.clone(), CallPartyStatus::Invited),
                contact,
            );
            self.build_call_state_messages(call)
        };
        for msg in msgs {
            self.net.send(msg);
        }
        self.calls_changed();
        Ok(())
    }

    fn build_call_state_messages(&self, call: &Call) -> Vec<HoshiMessage> {
        let my_key = self.public_key();
        call.non_hungup_party_keys()
            .into_iter()
            .filter(|k| k != &my_key)
            .map(|key| {
                HoshiMessage::new(
                    my_key.clone(),
                    key,
                    HoshiPayload::UpdateCallState {
                        call_id: call.id().to_string(),
                        events: call.events().clone(),
                    },
                )
            })
            .collect()
    }

    pub fn public_key(&self) -> String {
        self.public_key.borrow().clone()
    }

    pub fn own_contact(&self) -> Contact {
        self.contact_for(&self.public_key())
    }

    pub fn contact_for(&self, public_key: &str) -> Contact {
        self.contact_get(public_key)
            .unwrap_or_else(|| Contact::new(public_key.to_string()))
    }

    pub fn set_user_alias(&self, alias: &str) {
        if !self.nodes.borrow_mut().user_alias_set(alias) {
            return;
        }
        let contact_keys: Vec<String> = self.contacts.borrow().keys().cloned().collect();
        for key in &contact_keys {
            self.advertise_user(key);
        }
    }

    pub fn user_alias(&self, public_key: &str) -> Option<String> {
        self.nodes.borrow_mut().user_alias_get(public_key)
    }

    pub fn display_name(&self, public_key: &str) -> String {
        self.contact_for(public_key)
            .display_name(Some(&mut self.nodes.borrow_mut()))
    }

    pub fn set_public_key(&self, key: String) -> Result<()> {
        self.net.disconnect_all();
        self.nodes.borrow_mut().config_set("public_key", &key);
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

    pub fn last_message(&self, chat_id: &str) -> Option<ChatMessage> {
        let messages = self.messages.borrow();
        messages.get(chat_id)?.values().max().cloned()
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
                HoshiPayload::UpdateCallState { call_id, events } => {
                    // Auto-create unknown contact for the caller if not known
                    if self.contact_get(&net_msg.from_key).is_none() {
                        let contact = Contact::new_unknown(net_msg.from_key.clone());
                        let _ = self.contact_upsert(contact);
                    }
                    let mut found = false;
                    let contact_lookup = |key: &str| -> Contact { self.contact_for(key) };
                    for call in self.calls.borrow_mut().iter_mut() {
                        if call.id() == &call_id {
                            call.merge_events(events.clone(), &contact_lookup);
                            found = true;
                            break;
                        }
                    }
                    if !found {
                        let mut call = Call::from_events(call_id.clone(), events, &contact_lookup);
                        let own_status = call.get_status(&self.public_key());
                        if matches!(own_status, Some(CallPartyStatus::Invited)) {
                            // Transition from Invited to Ringing so other parties
                            // know we received the call
                            call.add_event(CallPartyEvent::new(
                                self.public_key(),
                                CallPartyStatus::Ringing,
                            ));
                            let msgs = self.build_call_state_messages(&call);
                            if let Some(interface) = self.audio_interface.borrow().as_ref() {
                                if let Ok(stream) = interface.create(self, &call) {
                                    call.set_audio(Some(stream));
                                }
                            }
                            self.calls.borrow_mut().push(call);
                            for msg in msgs {
                                self.net.send(msg);
                            }
                        } else {
                            // We're not ringing in this call — send HungUp immediately
                            let event =
                                CallPartyEvent::new(self.public_key(), CallPartyStatus::HungUp);
                            self.net.send(HoshiMessage::new(
                                self.public_key(),
                                net_msg.from_key.clone(),
                                HoshiPayload::UpdateCallState {
                                    call_id,
                                    events: vec![event],
                                },
                            ));
                        }
                    }
                    self.calls_changed();
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
                HoshiPayload::NodeAdvertise { path, hash } => {
                    if !self.is_node_interested(&path) {
                        continue;
                    }
                    let mut nodes = self.nodes.borrow_mut();
                    if !nodes.may_read(&net_msg.from_key, &path) {
                        continue;
                    }
                    if let Ok(remote_hash) = <[u8; 32]>::try_from(hash.as_slice()) {
                        let remote_hash = blake3::Hash::from(remote_hash);
                        let local_hash = nodes.get_hash(&path);
                        if local_hash != Some(remote_hash) {
                            drop(nodes);
                            self.node_sync(&net_msg.from_key, &path, local_hash.is_some());
                        }
                    }
                }
                HoshiPayload::NodePut {
                    path,
                    payload,
                    hash,
                } => {
                    if let Ok(mut node) = rmp_serde::from_slice::<HoshiNode>(&payload) {
                        node.path = path.clone();
                        let mut nodes = self.nodes.borrow_mut();
                        if nodes.may_write(&net_msg.from_key, &path, &node) {
                            nodes.insert(node);
                            if let Ok(h) = <[u8; 32]>::try_from(hash.as_slice()) {
                                nodes.set_hash(path.clone(), blake3::Hash::from(h));
                            }
                        }
                        drop(nodes);
                        if path.starts_with("/user/") {
                            self.contacts_changed();
                        }
                        if path.starts_with("/chat/") {
                            // Extract the chat path (/chat/{xor_hex}) from the full node path
                            let parts: Vec<&str> = path.splitn(4, '/').collect();
                            if parts.len() >= 3 {
                                let cp = format!("/{}/{}", parts[1], parts[2]);
                                if let Some(peer_key) = self.peer_key_for_chat_path(&cp) {
                                    // Auto-create unknown contact if peer isn't known
                                    if self.contact_get(&peer_key).is_none() {
                                        let contact = Contact::new_unknown(peer_key.clone());
                                        let _ = self.contact_upsert(contact);
                                    }
                                    self.rebuild_chat_messages(&cp, &peer_key);
                                }
                            }
                        }
                    }
                }
                HoshiPayload::NodeList { path } => {
                    let mut nodes = self.nodes.borrow_mut();
                    if !nodes.may_read(&net_msg.from_key, &path) {
                        continue;
                    }
                    let children: Vec<String> = nodes
                        .children(&path)
                        .iter()
                        .map(|n| n.path.clone())
                        .collect();
                    for child_path in children {
                        let h = nodes.hash(&child_path);
                        self.net.send(HoshiMessage::new(
                            self.public_key(),
                            net_msg.from_key.clone(),
                            HoshiPayload::NodeAdvertise {
                                path: child_path,
                                hash: h.as_bytes().to_vec(),
                            },
                        ));
                    }
                }
                HoshiPayload::NodeRequest { path } => {
                    let mut nodes = self.nodes.borrow_mut();
                    if !nodes.may_read(&net_msg.from_key, &path) {
                        continue;
                    }
                    // Send the node itself if it exists
                    if let Some(node) = nodes.get(&path) {
                        if let Ok(payload) = rmp_serde::to_vec(node) {
                            let h = nodes.hash(&path);
                            self.net.send(HoshiMessage::new(
                                self.public_key(),
                                net_msg.from_key.clone(),
                                HoshiPayload::NodePut {
                                    path: path.clone(),
                                    payload,
                                    hash: h.as_bytes().to_vec(),
                                },
                            ));
                        }
                    }
                    // Always advertise children, even for virtual parent paths
                    // that have no node themselves (e.g. /chat/{xor})
                    let child_paths: Vec<String> = nodes
                        .children(&path)
                        .iter()
                        .map(|n| n.path.clone())
                        .collect();
                    for child_path in child_paths {
                        let h = nodes.hash(&child_path);
                        self.net.send(HoshiMessage::new(
                            self.public_key(),
                            net_msg.from_key.clone(),
                            HoshiPayload::NodeAdvertise {
                                path: child_path,
                                hash: h.as_bytes().to_vec(),
                            },
                        ));
                    }
                }
                _ => {}
            }
        }

        let mut msgs = 0;

        let before = self.calls.borrow().len();
        self.calls.borrow_mut().retain_mut(|call| {
            msgs += 1;
            call.step(self);

            let own_status = call.get_status(&self.public_key());
            if own_status.is_none() || matches!(own_status, Some(CallPartyStatus::HungUp)) {
                false
            } else {
                call.active_or_ringing_party_count() > 1
            }
        });
        if self.calls.borrow().len() != before {
            self.calls_changed();
        }

        // Periodic sync: advertise one contact per tick (round-robin)
        let now = Instant::now();
        if now.duration_since(*self.last_sync.borrow()) > Duration::from_secs(5) {
            *self.last_sync.borrow_mut() = now;
            let contact_keys: Vec<String> = self.contacts.borrow().keys().cloned().collect();
            if !contact_keys.is_empty() {
                let idx = *self.sync_index.borrow() % contact_keys.len();
                *self.sync_index.borrow_mut() = idx + 1;
                self.advertise_chat(&contact_keys[idx]);
                self.advertise_user(&contact_keys[idx]);
                self.sync_peer_user(&contact_keys[idx]);
            }
        }

        msgs
    }

    pub fn message_upsert(&self, msg: ChatMessage) -> Result<()> {
        let cp = msg.chat_id();
        let msg_uuid = uuid::Uuid::now_v7().to_string();
        let text_uuid = uuid::Uuid::now_v7().to_string();
        let msg_path = format!("{cp}/{msg_uuid}");
        let text_path = format!("{msg_path}/{text_uuid}");

        {
            let mut nodes = self.nodes.borrow_mut();
            nodes.insert(HoshiNode {
                from: msg.from.clone(),
                path: msg_path,
                payload: HoshiNodePayload::Message,
            });
            nodes.insert(HoshiNode {
                from: msg.from.clone(),
                path: text_path,
                payload: HoshiNodePayload::ChatText {
                    content: msg.content.clone(),
                },
            });
        }

        self.advertise_chat(&msg.to);
        self.rebuild_chat_messages(&cp, &msg.to);

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
        F: FnOnce(&Self, &HashMap<String, Contact>),
    {
        let contacts = self.contacts.borrow();
        f(self, &contacts);
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
        self.advertise_chat(&contact.public_key);
        if contact.contact_type != crate::ContactType::Blocked {
            self.node_interest_add(&user_path(&contact.public_key));
            self.sync_peer_user(&contact.public_key);
        } else {
            self.node_interest_remove(&user_path(&contact.public_key));
        }
        self.nodes.borrow_mut().contact_upsert(&contact);
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
        self.node_interest_remove(&user_path(public_key));
        self.nodes.borrow_mut().contact_delete(public_key);
        self.contacts_changed();

        Ok(())
    }
}

impl std::fmt::Debug for HoshiClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HoshiClient")
            .field("contacts", &self.contacts)
            .field(
                "contacts_watchers",
                &format!("[{} watchers]", self.contacts_watchers.borrow().len()),
            )
            .finish()
    }
}
