use std::{cell::RefCell, collections::HashMap, fs, path::Path, path::PathBuf, rc::Rc};

use anyhow::{Result, anyhow};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{
    AudioInterface, Call, ChatMessage, Contact, ContactType, HeadCommand, HoshiNetClient, Store,
    StoreHead,
    call::{CallPartyEvent, CallPartyStatus},
    chat_path,
    hoshi_net_client::{HoshiMessage, HoshiPayload},
    identity::HoshiIdentity,
    peer_key_from_chat_path, user_path,
};

const CONTACTS_HEAD: &str = "local_contacts";
const CONFIG_HEAD: &str = "local_config";
const CONFIG_KEY_PUBLIC_KEY: &str = "public_key";

#[derive(Clone, Debug, Serialize, Deserialize)]
struct ChatRecord {
    id: Uuid,
    from: String,
    content: String,
}

impl Store for ChatRecord {
    fn id(&self) -> Uuid {
        self.id
    }

    fn hash(&self) -> blake3::Hash {
        blake3::Hasher::new()
            .update(self.id.as_bytes())
            .update(self.from.as_bytes())
            .update(self.content.as_bytes())
            .finalize()
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct ContactRecord {
    id: Uuid,
    public_key: String,
    contact_type: ContactType,
}

impl Store for ContactRecord {
    fn id(&self) -> Uuid {
        self.id
    }

    fn hash(&self) -> blake3::Hash {
        blake3::Hasher::new()
            .update(self.id.as_bytes())
            .update(self.public_key.as_bytes())
            .update(format!("{:?}", self.contact_type).as_bytes())
            .finalize()
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct ProfileRecord {
    id: Uuid,
    alias: String,
}

impl Store for ProfileRecord {
    fn id(&self) -> Uuid {
        self.id
    }

    fn hash(&self) -> blake3::Hash {
        blake3::Hasher::new()
            .update(self.id.as_bytes())
            .update(self.alias.as_bytes())
            .finalize()
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct ConfigRecord {
    id: Uuid,
    key: String,
    value: String,
}

struct ContactWatcher {
    id: Uuid,
    fun: Box<dyn Fn(&HoshiClient, &HashMap<String, Contact>)>,
}

pub struct HoshiContactsWatcherRef {
    id: Uuid,
    watchers: Rc<RefCell<Vec<ContactWatcher>>>,
}

impl Drop for HoshiContactsWatcherRef {
    fn drop(&mut self) {
        self.watchers
            .borrow_mut()
            .retain(|watcher| watcher.id != self.id);
    }
}

struct MessageWatcher {
    id: Uuid,
    filter: String,
    fun: Box<dyn Fn(&HoshiClient, &str, &HashMap<String, ChatMessage>)>,
}

pub struct HoshiMessagesWatcherRef {
    id: Uuid,
    watchers: Rc<RefCell<Vec<MessageWatcher>>>,
}

impl Drop for HoshiMessagesWatcherRef {
    fn drop(&mut self) {
        self.watchers
            .borrow_mut()
            .retain(|watcher| watcher.id != self.id);
    }
}

impl Store for ConfigRecord {
    fn id(&self) -> Uuid {
        self.id
    }

    fn hash(&self) -> blake3::Hash {
        blake3::Hasher::new()
            .update(self.id.as_bytes())
            .update(self.key.as_bytes())
            .update(self.value.as_bytes())
            .finalize()
    }
}

fn read_seed_hex(path: &Path) -> Option<String> {
    fs::read_to_string(path)
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

fn write_identity_files(path: &Path, identity: &HoshiIdentity) -> Result<()> {
    let seed_hex: String = identity
        .seed()
        .iter()
        .map(|b| format!("{:02x}", b))
        .collect();
    fs::write(path, format!("{seed_hex}\n"))?;
    fs::write(
        path.with_extension("pub"),
        format!("{}\n", identity.public_key_hex()),
    )?;
    Ok(())
}

fn load_or_create_identity(path: &Path) -> Result<HoshiIdentity> {
    if let Some(seed_hex) = read_seed_hex(path) {
        let seed_bytes: Vec<u8> = (0..seed_hex.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&seed_hex[i..i + 2], 16).unwrap())
            .collect();
        let seed: [u8; 32] = seed_bytes
            .try_into()
            .expect("ed25519_seed must be 32 bytes");
        Ok(HoshiIdentity::from_seed(seed))
    } else {
        let identity = HoshiIdentity::generate();
        write_identity_files(path, &identity)?;
        Ok(identity)
    }
}

fn config_get(head: &StoreHead<ConfigRecord>, key: &str) -> Option<String> {
    head.get_all()
        .values()
        .filter(|entry| entry.key == key)
        .map(|entry| entry.value.clone())
        .last()
}

fn contacts_from_head(head: &StoreHead<ContactRecord>) -> HashMap<String, Contact> {
    let mut contacts = HashMap::new();
    for record in head.get_all().values() {
        if record.contact_type == ContactType::Deleted {
            contacts.remove(&record.public_key);
            continue;
        }
        let mut contact = Contact::new(record.public_key.clone());
        contact.contact_type = record.contact_type.clone();
        contacts.insert(contact.public_key.clone(), contact);
    }
    contacts
}

fn alias_from_head(head: &StoreHead<ProfileRecord>) -> Option<String> {
    head.get_all()
        .values()
        .last()
        .map(|record| record.alias.clone())
}

fn chat_messages_from_head(
    head: &StoreHead<ChatRecord>,
    our_key: &str,
    peer_key: &str,
) -> HashMap<String, ChatMessage> {
    let mut result = HashMap::new();
    for record in head.get_all().values() {
        let created_at = record
            .id
            .get_timestamp()
            .map(|ts| ts.to_unix().0 as i64)
            .unwrap_or(0);
        let to = if record.from == our_key {
            peer_key.to_string()
        } else {
            our_key.to_string()
        };
        result.insert(
            record.id.to_string(),
            ChatMessage::new(
                record.id.to_string(),
                created_at,
                record.from.clone(),
                to,
                record.content.clone(),
            ),
        );
    }
    result
}

pub struct HoshiClient {
    pub(crate) net: HoshiNetClient,
    store_dir: PathBuf,
    public_key: RefCell<String>,

    pub(crate) audio_interface: RefCell<Option<Box<dyn AudioInterface>>>,

    calls: RefCell<Vec<Call>>,
    calls_watchers: RefCell<Vec<Box<dyn Fn(&Self, &Vec<Call>)>>>,

    contacts_head: RefCell<StoreHead<ContactRecord>>,
    contacts: RefCell<HashMap<String, Contact>>,
    contacts_watchers: Rc<RefCell<Vec<ContactWatcher>>>,

    config_head: RefCell<StoreHead<ConfigRecord>>,
    profile_heads: RefCell<HashMap<String, StoreHead<ProfileRecord>>>,
    aliases: RefCell<HashMap<String, String>>,

    chats: RefCell<HashMap<String, StoreHead<ChatRecord>>>,
    messages: RefCell<HashMap<String, HashMap<String, ChatMessage>>>,
    messages_watchers: Rc<RefCell<Vec<MessageWatcher>>>,
}

impl HoshiClient {
    pub fn new(data_dir: Option<PathBuf>) -> Result<Self> {
        let data_dir = data_dir.unwrap_or_else(|| dirs::home_dir().unwrap().join(".hoshi"));
        fs::create_dir_all(&data_dir)?;

        let store_dir = data_dir.join("stores");
        fs::create_dir_all(&store_dir)?;

        let identity = load_or_create_identity(&data_dir.join("hoshi_ed25519"))?;
        let default_public_key = identity.public_key_hex();

        let config_head = StoreHead::<ConfigRecord>::new(CONFIG_HEAD.to_string(), Some(&store_dir));
        let public_key = config_get(&config_head, CONFIG_KEY_PUBLIC_KEY)
            .unwrap_or_else(|| default_public_key.clone());

        let tls_config = identity.make_client_tls_config();
        let net = HoshiNetClient::new(tls_config);

        let contacts_head =
            StoreHead::<ContactRecord>::new(CONTACTS_HEAD.to_string(), Some(&store_dir));
        let contacts = contacts_from_head(&contacts_head);

        let mut profile_heads = HashMap::new();
        profile_heads.insert(
            user_path(&public_key),
            StoreHead::<ProfileRecord>::new(user_path(&public_key), Some(&store_dir)),
        );

        let mut chats = HashMap::new();
        for contact in contacts.values() {
            let chat_id = chat_path(&public_key, &contact.public_key);
            let mut chat_head = StoreHead::<ChatRecord>::new(chat_id.clone(), Some(&store_dir));
            chat_head.remote_add(contact.public_key.clone(), None);
            chats.insert(chat_id, chat_head);

            profile_heads
                .entry(user_path(&contact.public_key))
                .or_insert_with(|| {
                    StoreHead::<ProfileRecord>::new(
                        user_path(&contact.public_key),
                        Some(&store_dir),
                    )
                });
        }

        if let Some(own_profile) = profile_heads.get_mut(&user_path(&public_key)) {
            for contact in contacts.values() {
                if contact.contact_type != ContactType::Blocked {
                    own_profile.remote_add(contact.public_key.clone(), None);
                }
            }
        }

        let aliases = profile_heads
            .iter()
            .filter_map(|(path, head)| {
                let public_key = path.strip_prefix("/user/")?;
                Some((public_key.to_string(), alias_from_head(head)?))
            })
            .collect::<HashMap<_, _>>();

        let messages = chats
            .iter()
            .filter_map(|(chat_id, head)| {
                let peer_key = peer_key_from_chat_path(&public_key, chat_id)?;
                let msgs = chat_messages_from_head(head, &public_key, &peer_key);
                if msgs.is_empty() {
                    None
                } else {
                    Some((chat_id.clone(), msgs))
                }
            })
            .collect::<HashMap<_, _>>();

        let relay_list = if cfg!(debug_assertions) {
            vec!["wss://127.0.0.1:2800/".into()]
        } else {
            vec!["wss://hoshi.rubhub.net:2800/".into()]
        };
        net.update_relays(&relay_list);

        Ok(Self {
            net,
            store_dir,
            public_key: RefCell::new(public_key),
            audio_interface: RefCell::new(None),
            calls: RefCell::new(vec![]),
            calls_watchers: RefCell::new(vec![]),
            contacts_head: RefCell::new(contacts_head),
            contacts: RefCell::new(contacts),
            contacts_watchers: Rc::new(RefCell::new(vec![])),
            config_head: RefCell::new(config_head),
            profile_heads: RefCell::new(profile_heads),
            aliases: RefCell::new(aliases),
            chats: RefCell::new(chats),
            messages: RefCell::new(messages),
            messages_watchers: Rc::new(RefCell::new(vec![])),
        })
    }

    pub fn set_audio_interface(&self, interface: Option<Box<dyn AudioInterface>>) {
        *self.audio_interface.borrow_mut() = interface;
    }

    fn ensure_own_profile_head(&self) {
        let head_name = user_path(&self.public_key());
        let contact_keys = self
            .contacts
            .borrow()
            .values()
            .filter(|c| c.contact_type != ContactType::Blocked)
            .map(|c| c.public_key.clone())
            .collect::<Vec<_>>();
        let mut profiles = self.profile_heads.borrow_mut();
        let head = profiles
            .entry(head_name.clone())
            .or_insert_with(|| StoreHead::<ProfileRecord>::new(head_name, Some(&self.store_dir)));
        for key in contact_keys {
            head.remote_add(key, None);
        }
    }

    fn ensure_profile_head(&self, public_key: &str) {
        let head_name = user_path(public_key);
        self.profile_heads
            .borrow_mut()
            .entry(head_name.clone())
            .or_insert_with(|| StoreHead::<ProfileRecord>::new(head_name, Some(&self.store_dir)));
    }

    fn ensure_chat_head(&self, chat_id: &str) {
        let peer_key = peer_key_from_chat_path(&self.public_key(), chat_id);
        self.chats
            .borrow_mut()
            .entry(chat_id.to_string())
            .or_insert_with(|| {
                let mut head =
                    StoreHead::<ChatRecord>::new(chat_id.to_string(), Some(&self.store_dir));
                if let Some(peer_key) = peer_key {
                    head.remote_add(peer_key, None);
                }
                head
            });
    }

    fn sync_head<T: Store>(&self, head_name: &str, head: &mut StoreHead<T>) {
        let from_key = self.public_key();
        let head_name = head_name.to_string();
        head.tx(|dest, cmd| {
            if let Ok(command) = rmp_serde::to_vec(&cmd) {
                self.net.send(HoshiMessage::new(
                    from_key.clone(),
                    dest,
                    HoshiPayload::StoreSync {
                        head_name: head_name.clone(),
                        command,
                    },
                ));
            }
        });
    }

    fn rebuild_views(&self) {
        let new_contacts = {
            let contacts_head = self.contacts_head.borrow();
            contacts_from_head(&contacts_head)
        };

        let new_aliases = {
            let profiles = self.profile_heads.borrow();
            profiles
                .iter()
                .filter_map(|(path, head)| {
                    let public_key = path.strip_prefix("/user/")?;
                    Some((public_key.to_string(), alias_from_head(head)?))
                })
                .collect::<HashMap<_, _>>()
        };

        let new_messages = {
            let chats = self.chats.borrow();
            let our_key = self.public_key();
            chats
                .iter()
                .filter_map(|(chat_id, head)| {
                    let peer_key = peer_key_from_chat_path(&our_key, chat_id)?;
                    let msgs = chat_messages_from_head(head, &our_key, &peer_key);
                    if msgs.is_empty() {
                        None
                    } else {
                        Some((chat_id.clone(), msgs))
                    }
                })
                .collect::<HashMap<_, _>>()
        };

        let aliases_changed = {
            let aliases = self.aliases.borrow();
            *aliases != new_aliases
        };
        if aliases_changed {
            *self.aliases.borrow_mut() = new_aliases;
        }

        let contacts_changed = {
            let contacts = self.contacts.borrow();
            *contacts != new_contacts
        };
        if contacts_changed {
            *self.contacts.borrow_mut() = new_contacts;
        }
        if aliases_changed || contacts_changed {
            self.contacts_changed();
        }

        let old_messages = self.messages.borrow().clone();
        if old_messages != new_messages {
            *self.messages.borrow_mut() = new_messages.clone();
            let mut changed_chat_ids = old_messages.keys().cloned().collect::<Vec<_>>();
            for key in new_messages.keys() {
                if !changed_chat_ids.contains(key) {
                    changed_chat_ids.push(key.clone());
                }
            }
            for chat_id in changed_chat_ids {
                if old_messages.get(&chat_id) != new_messages.get(&chat_id) {
                    self.messages_changed(chat_id);
                }
            }
        }
    }

    /// Derive the peer key for a chat path from the XOR hash
    pub fn peer_key_for_chat_path(&self, cp: &str) -> Option<String> {
        peer_key_from_chat_path(&self.public_key(), cp)
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

    fn contact_for(&self, public_key: &str) -> Contact {
        self.contact_get(public_key)
            .unwrap_or_else(|| Contact::new(public_key.to_string()))
    }

    pub fn set_user_alias(&self, alias: &str) {
        self.ensure_own_profile_head();
        let head_name = user_path(&self.public_key());
        let profiles = self.profile_heads.borrow_mut();
        if let Some(head) = profiles.get(&head_name)
            && alias_from_head(head) == Some(alias.to_string())
        {
            return;
        }
        drop(profiles);
        let mut profiles = self.profile_heads.borrow_mut();
        if let Some(head) = profiles.get_mut(&head_name) {
            head.queue(ProfileRecord {
                id: Uuid::now_v7(),
                alias: alias.to_string(),
            });
        }
    }

    pub fn user_alias(&self, public_key: &str) -> Option<String> {
        self.aliases.borrow().get(public_key).cloned()
    }

    pub fn display_name(&self, public_key: &str) -> String {
        self.user_alias(public_key)
            .unwrap_or_else(|| self.contact_for(public_key).display_name())
    }

    pub fn set_public_key(&self, key: String) -> Result<()> {
        self.net.disconnect_all();
        self.config_head.borrow_mut().insert(ConfigRecord {
            id: Uuid::now_v7(),
            key: CONFIG_KEY_PUBLIC_KEY.to_string(),
            value: key.clone(),
        });
        *self.public_key.borrow_mut() = key;
        self.ensure_own_profile_head();
        self.rebuild_views();
        Ok(())
    }

    fn contacts_changed(&self) {
        let watchers = self.contacts_watchers.borrow();
        for watcher in &*watchers {
            let contacts = self.contacts.borrow();
            (watcher.fun)(self, &contacts);
        }
    }

    pub fn last_message(&self, chat_id: &str) -> Option<ChatMessage> {
        let messages = self.messages.borrow();
        messages.get(chat_id)?.values().max().cloned()
    }

    fn messages_changed(&self, chat_id: String) {
        let watchers = self.messages_watchers.borrow();
        for watcher in &*watchers {
            if watcher.filter.is_empty() || watcher.filter == chat_id {
                let messages = self.messages.borrow();
                let messages = messages.get(&chat_id);
                if let Some(messages) = messages {
                    (watcher.fun)(self, &chat_id, messages);
                }
            }
        }
    }

    /// Main function a client MUST call regularly for the clientlib
    /// to work as expected. It communicates with the various other
    /// threads and updates the internal state as well as fires callbacks.
    pub fn step(&self) -> u32 {
        let mut store_changed = false;

        if self.contacts_head.borrow_mut().step() > 0 {
            store_changed = true;
        }
        if self.config_head.borrow_mut().step() > 0 {
            store_changed = true;
        }
        {
            let mut profiles = self.profile_heads.borrow_mut();
            for head in profiles.values_mut() {
                if head.step() > 0 {
                    store_changed = true;
                }
            }
        }
        {
            let mut chats = self.chats.borrow_mut();
            for head in chats.values_mut() {
                if head.step() > 0 {
                    store_changed = true;
                }
            }
        }

        let mut msgs = 0;
        for net_msg in self.net.step() {
            msgs += 1;
            match net_msg.payload {
                HoshiPayload::UpdateCallState { call_id, events } => {
                    if self.contact_get(&net_msg.from_key).is_none() {
                        let _ = self.contact_upsert(Contact::new_unknown(net_msg.from_key.clone()));
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
                HoshiPayload::StoreSync { head_name, command } => {
                    if head_name.starts_with("/chat/") {
                        if self.peer_key_for_chat_path(&head_name) != Some(net_msg.from_key.clone())
                        {
                            continue;
                        }
                        self.ensure_chat_head(&head_name);
                        if self.contact_get(&net_msg.from_key).is_none() {
                            let _ =
                                self.contact_upsert(Contact::new_unknown(net_msg.from_key.clone()));
                        }
                        if let Ok(cmd) = rmp_serde::from_slice::<HeadCommand<ChatRecord>>(&command)
                        {
                            let mut chats = self.chats.borrow_mut();
                            if let Some(head) = chats.get_mut(&head_name)
                                && head.rx(&net_msg.from_key, cmd)
                            {
                                store_changed = true;
                            }
                        }
                    } else if head_name.starts_with("/user/") {
                        if head_name != user_path(&net_msg.from_key) {
                            continue;
                        }
                        self.ensure_profile_head(&net_msg.from_key);
                        if let Ok(cmd) =
                            rmp_serde::from_slice::<HeadCommand<ProfileRecord>>(&command)
                        {
                            let mut profiles = self.profile_heads.borrow_mut();
                            if let Some(head) = profiles.get_mut(&head_name)
                                && head.rx(&net_msg.from_key, cmd)
                            {
                                store_changed = true;
                            }
                        }
                    }
                }
                _ => {}
            }
        }

        if self.contacts_head.borrow_mut().step() > 0 {
            store_changed = true;
        }
        {
            let mut profiles = self.profile_heads.borrow_mut();
            for head in profiles.values_mut() {
                if head.step() > 0 {
                    store_changed = true;
                }
            }
        }
        {
            let mut chats = self.chats.borrow_mut();
            for head in chats.values_mut() {
                if head.step() > 0 {
                    store_changed = true;
                }
            }
        }

        {
            let own_head = user_path(&self.public_key());
            let mut profiles = self.profile_heads.borrow_mut();
            for (head_name, head) in profiles.iter_mut() {
                if head_name == &own_head || head_name.starts_with("/user/") {
                    self.sync_head(head_name, head);
                }
            }
        }
        {
            let mut chats = self.chats.borrow_mut();
            for (chat_id, head) in chats.iter_mut() {
                self.sync_head(chat_id, head);
            }
        }

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

        if store_changed {
            self.rebuild_views();
        }

        msgs
    }

    pub fn message_upsert(&self, msg: ChatMessage) -> Result<()> {
        let chat_id = msg.chat_id();
        self.ensure_chat_head(&chat_id);
        if let Some(peer_key) = self.peer_key_for_chat_path(&chat_id) {
            let mut chats = self.chats.borrow_mut();
            if let Some(head) = chats.get_mut(&chat_id) {
                head.remote_add(peer_key, None);
                let id = Uuid::parse_str(&msg.id).unwrap_or_else(|_| Uuid::now_v7());
                head.queue(ChatRecord {
                    id,
                    from: msg.from.clone(),
                    content: msg.content.clone(),
                });
            }
        }
        Ok(())
    }

    /// Call this function to get notified whenever messages in a
    /// particular chat_id as specified by filter changes, can also
    /// be left empty to get notified about all messages.
    #[inline]
    pub fn messages_watch<F>(&self, filter: String, f: F) -> HoshiMessagesWatcherRef
    where
        F: Fn(&HoshiClient, &str, &HashMap<String, ChatMessage>) + 'static,
    {
        {
            let chats = self.messages.borrow();
            if filter.is_empty() {
                for chat_id in chats.keys() {
                    if let Some(chat) = chats.get(chat_id) {
                        f(self, chat_id, chat);
                    }
                }
            } else if let Some(chat) = chats.get(&filter) {
                f(self, &filter, chat);
            }
        }

        let id = Uuid::now_v7();
        self.messages_watchers.borrow_mut().push(MessageWatcher {
            id,
            filter,
            fun: Box::new(f),
        });
        HoshiMessagesWatcherRef {
            id,
            watchers: self.messages_watchers.clone(),
        }
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
    #[inline]
    pub fn contacts_watch<F>(&self, f: F) -> HoshiContactsWatcherRef
    where
        F: Fn(&HoshiClient, &HashMap<String, Contact>) + 'static,
    {
        let contacts = self.contacts.borrow();
        f(self, &contacts);
        let id = Uuid::now_v7();
        self.contacts_watchers.borrow_mut().push(ContactWatcher {
            id,
            fun: Box::new(f),
        });
        HoshiContactsWatcherRef {
            id,
            watchers: self.contacts_watchers.clone(),
        }
    }

    /// Lookup a particular public_key in the current snapshot of Contacts
    #[inline]
    pub fn contact_get(&self, public_key: &str) -> Option<Contact> {
        self.contacts.borrow().get(public_key).cloned()
    }

    /// Update or Insert a particular Contact, currently only persists locally.
    pub fn contact_upsert(&self, contact: Contact) -> Result<()> {
        self.contacts_head.borrow().queue(ContactRecord {
            id: Uuid::now_v7(),
            public_key: contact.public_key.clone(),
            contact_type: contact.contact_type.clone(),
        });

        let chat_id = chat_path(&self.public_key(), &contact.public_key);
        self.ensure_chat_head(&chat_id);
        {
            let mut chats = self.chats.borrow_mut();
            if let Some(head) = chats.get_mut(&chat_id) {
                head.remote_add(contact.public_key.clone(), None);
            }
        }

        self.ensure_profile_head(&contact.public_key);
        let own_path = user_path(&self.public_key());
        {
            let mut profiles = self.profile_heads.borrow_mut();
            let head = profiles.entry(own_path.clone()).or_insert_with(|| {
                StoreHead::<ProfileRecord>::new(own_path.clone(), Some(&self.store_dir))
            });
            if contact.contact_type == ContactType::Blocked {
                head.remote_drop(&contact.public_key);
            } else {
                head.remote_add(contact.public_key.clone(), None);
            }
        }

        Ok(())
    }

    /// Remove a contact locally.
    pub fn contact_delete(&self, public_key: &str) -> Result<()> {
        self.contacts_head.borrow().queue(ContactRecord {
            id: Uuid::now_v7(),
            public_key: public_key.to_string(),
            contact_type: ContactType::Deleted,
        });
        let own_path = user_path(&self.public_key());
        let mut profiles = self.profile_heads.borrow_mut();
        if let Some(head) = profiles.get_mut(&own_path) {
            head.remote_drop(public_key);
        }

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
