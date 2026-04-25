use std::{
    cell::RefCell,
    collections::{HashMap, HashSet},
    fs,
    path::{Path, PathBuf},
    rc::Rc,
};

use anyhow::{Context, Result, anyhow};
use uuid::Uuid;

use crate::{
    AudioInterface, Call, ChatMessage, Contact, ContactType, HeadCommand, HoshiNetClient,
    HoshiRecord, HoshiSignedRecord, StoreHead,
    call::{CallPartyEvent, CallPartyStatus},
    chat_path,
    hoshi_net_client::{HoshiMessage, HoshiNetPayload},
    identity::HoshiIdentity,
    normalize_public_key, peer_key_from_chat_path,
    record::HoshiPayload,
    user_path, validate_public_key_hex,
};

const CONTACTS_HEAD: &str = "local_contacts";
const CONFIG_HEAD: &str = "local_config";
const CONFIG_KEY_PUBLIC_KEY: &str = "public_key";
const CALL_END_GRACE_SECS: u64 = 5;

type ContactWatchFn = Rc<dyn Fn(&HoshiClient, &HashMap<String, Contact>)>;
type MessageWatchFn = Rc<dyn Fn(&HoshiClient, &str, &HashMap<String, ChatMessage>)>;
type CallWatchFn = Rc<dyn Fn(&HoshiClient, &[Call])>;

struct Watcher<F: ?Sized> {
    id: Uuid,
    fun: Rc<F>,
}

type ContactWatcher = Watcher<dyn Fn(&HoshiClient, &HashMap<String, Contact>)>;
type CallWatcher = Watcher<dyn Fn(&HoshiClient, &[Call])>;

pub struct HoshiWatchRef {
    cleanup: Option<Box<dyn FnMut()>>,
}

impl HoshiWatchRef {
    fn new(cleanup: impl FnMut() + 'static) -> Self {
        Self {
            cleanup: Some(Box::new(cleanup)),
        }
    }
}

impl Drop for HoshiWatchRef {
    fn drop(&mut self) {
        if let Some(mut cleanup) = self.cleanup.take() {
            cleanup();
        }
    }
}

struct MessageWatcher {
    id: Uuid,
    filter: Option<String>,
    fun: MessageWatchFn,
}

#[derive(Default)]
struct StepChanges {
    contacts_changed: bool,
    profiles_changed: bool,
    changed_chat_ids: HashSet<String>,
}

impl StepChanges {
    fn note_contacts(&mut self) {
        self.contacts_changed = true;
    }

    fn note_profiles(&mut self) {
        self.profiles_changed = true;
    }

    fn note_chat(&mut self, chat_id: String) {
        self.changed_chat_ids.insert(chat_id);
    }

    fn merge(&mut self, other: Self) {
        self.contacts_changed |= other.contacts_changed;
        self.profiles_changed |= other.profiles_changed;
        self.changed_chat_ids.extend(other.changed_chat_ids);
    }

    fn has_contact_view_changes(&self) -> bool {
        self.contacts_changed || self.profiles_changed
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
        if seed_hex.len() != 64 {
            return Err(anyhow!(
                "invalid ed25519 seed length in {}: expected 64 hex chars, got {}",
                path.display(),
                seed_hex.len()
            ));
        }
        let mut seed = [0_u8; 32];
        for (idx, offset) in (0..seed_hex.len()).step_by(2).enumerate() {
            seed[idx] = u8::from_str_radix(&seed_hex[offset..offset + 2], 16)
                .with_context(|| format!("invalid ed25519 seed hex in {}", path.display()))?;
        }
        Ok(HoshiIdentity::from_seed(seed))
    } else {
        let identity = HoshiIdentity::generate();
        write_identity_files(path, &identity)?;
        Ok(identity)
    }
}

fn config_get(head: &StoreHead<HoshiSignedRecord>, key: &str) -> Option<String> {
    head.get_all()
        .values()
        .filter(|entry| entry.verify())
        .filter_map(|entry| match &entry.record.payload {
            HoshiPayload::Config {
                key: entry_key,
                value,
            } if entry_key == key => Some(value.clone()),
            _ => None,
        })
        .last()
}

fn contacts_from_head(head: &StoreHead<HoshiSignedRecord>) -> HashMap<String, Contact> {
    let mut contacts = HashMap::new();
    for record in head.get_all().values() {
        if !record.verify() {
            continue;
        }
        let HoshiPayload::Contact {
            public_key,
            contact_type,
        } = &record.record.payload
        else {
            continue;
        };
        if contact_type == &ContactType::Deleted {
            contacts.remove(public_key);
        } else {
            let mut contact = Contact::new(public_key.clone());
            contact.contact_type = contact_type.clone();
            contacts.insert(contact.public_key.clone(), contact);
        }
    }
    contacts
}

fn alias_from_head(head: &StoreHead<HoshiSignedRecord>, public_key: &str) -> Option<String> {
    head.get_all()
        .values()
        .filter(|record| record.verify())
        .filter(|record| record.record.from == public_key)
        .filter_map(|record| match &record.record.payload {
            HoshiPayload::Title { title } => Some(title.clone()),
            _ => None,
        })
        .last()
}

fn chat_messages_from_head(
    head: &StoreHead<HoshiSignedRecord>,
    our_key: &str,
    peer_key: &str,
) -> HashMap<String, ChatMessage> {
    let mut result = HashMap::new();
    for record in head.get_all().values() {
        if !record.verify() {
            continue;
        }
        if record.record.from != our_key && record.record.from != peer_key {
            continue;
        }
        let HoshiPayload::Text { content } = &record.record.payload else {
            continue;
        };
        let created_at = record
            .record
            .id
            .get_timestamp()
            .map(|ts| ts.to_unix().0 as i64)
            .unwrap_or(0);
        let to = if record.record.from == our_key {
            peer_key.to_string()
        } else {
            our_key.to_string()
        };
        result.insert(
            record.record.id.to_string(),
            ChatMessage::new(
                record.record.id.to_string(),
                created_at,
                record.record.from.clone(),
                to,
                content.clone(),
            ),
        );
    }
    result
}

fn signed_store_head(name: String, store_dir: &Path) -> StoreHead<HoshiSignedRecord> {
    StoreHead::<HoshiSignedRecord>::new(name, Some(store_dir))
}

fn register_watch<F: ?Sized + 'static>(
    watchers: &Rc<RefCell<Vec<Watcher<F>>>>,
    fun: Rc<F>,
) -> HoshiWatchRef {
    let id = Uuid::now_v7();
    watchers.borrow_mut().push(Watcher { id, fun });
    let watchers = Rc::clone(watchers);
    HoshiWatchRef::new(move || {
        watchers.borrow_mut().retain(|watcher| watcher.id != id);
    })
}

fn configure_contact_heads(
    public_key: &str,
    store_dir: &Path,
    chats: &mut HashMap<String, StoreHead<HoshiSignedRecord>>,
    profiles: &mut HashMap<String, StoreHead<HoshiSignedRecord>>,
    contact: &Contact,
    request_profile_tip: bool,
) {
    let chat_id = chat_path(public_key, &contact.public_key);
    chats
        .entry(chat_id.clone())
        .or_insert_with(|| signed_store_head(chat_id, store_dir))
        .remote_add(contact.public_key.clone(), None);

    let contact_profile_path = user_path(&contact.public_key);
    let contact_profile = profiles
        .entry(contact_profile_path.clone())
        .or_insert_with(|| signed_store_head(contact_profile_path, store_dir));
    if contact.contact_type == ContactType::Blocked {
        contact_profile.remote_drop(&contact.public_key);
    } else if request_profile_tip {
        contact_profile.remote_add_with_tip_request(contact.public_key.clone(), None);
    } else {
        contact_profile.remote_add(contact.public_key.clone(), None);
    }

    let own_profile_path = user_path(public_key);
    let own_profile = profiles
        .entry(own_profile_path.clone())
        .or_insert_with(|| signed_store_head(own_profile_path, store_dir));
    if contact.contact_type == ContactType::Blocked {
        own_profile.remote_drop(&contact.public_key);
    } else {
        own_profile.remote_add(contact.public_key.clone(), None);
    }
}

pub struct HoshiClient {
    pub(crate) net: HoshiNetClient,
    store_dir: PathBuf,
    identity: HoshiIdentity,
    public_key: RefCell<String>,

    pub(crate) audio_interface: RefCell<Option<Box<dyn AudioInterface>>>,

    calls: RefCell<Vec<Call>>,
    calls_watchers: Rc<RefCell<Vec<CallWatcher>>>,

    contacts_head: RefCell<StoreHead<HoshiSignedRecord>>,
    contacts_watchers: Rc<RefCell<Vec<ContactWatcher>>>,

    config_head: RefCell<StoreHead<HoshiSignedRecord>>,
    profile_heads: RefCell<HashMap<String, StoreHead<HoshiSignedRecord>>>,

    chats: RefCell<HashMap<String, StoreHead<HoshiSignedRecord>>>,
    messages_watchers: Rc<RefCell<Vec<MessageWatcher>>>,
}

impl HoshiClient {
    pub fn new(data_dir: Option<PathBuf>) -> Result<Self> {
        let data_dir = match data_dir {
            Some(path) => path,
            None => dirs::home_dir()
                .map(|path| path.join(".hoshi"))
                .ok_or_else(|| anyhow!("couldn't determine home directory for hoshi data dir"))?,
        };
        fs::create_dir_all(&data_dir)?;

        let store_dir = data_dir.join("stores");
        fs::create_dir_all(&store_dir)?;

        let identity = load_or_create_identity(&data_dir.join("hoshi_ed25519"))?;
        let default_public_key = identity.public_key_hex();

        let config_head =
            StoreHead::<HoshiSignedRecord>::new(CONFIG_HEAD.to_string(), Some(&store_dir));
        let configured_public_key = config_get(&config_head, CONFIG_KEY_PUBLIC_KEY);
        let public_key = configured_public_key
            .filter(|key| key == &default_public_key)
            .unwrap_or_else(|| default_public_key.clone());

        let tls_config = identity.make_client_tls_config();
        let net = HoshiNetClient::new(tls_config);

        let contacts_head =
            StoreHead::<HoshiSignedRecord>::new(CONTACTS_HEAD.to_string(), Some(&store_dir));
        let contacts = contacts_from_head(&contacts_head);

        let mut profile_heads = HashMap::new();
        profile_heads.insert(
            user_path(&public_key),
            signed_store_head(user_path(&public_key), &store_dir),
        );

        let mut chats = HashMap::new();
        for contact in contacts.values() {
            configure_contact_heads(
                &public_key,
                &store_dir,
                &mut chats,
                &mut profile_heads,
                contact,
                true,
            );
        }

        let relay_list = if cfg!(debug_assertions) {
            vec!["wss://127.0.0.1:2800/".into()]
        } else {
            vec!["wss://hoshi.rubhub.net:2800/".into()]
        };
        net.update_relays(&relay_list);

        Ok(Self {
            net,
            store_dir,
            identity,
            public_key: RefCell::new(public_key),
            audio_interface: RefCell::new(None),
            calls: RefCell::new(vec![]),
            calls_watchers: Rc::new(RefCell::new(vec![])),
            contacts_head: RefCell::new(contacts_head),
            contacts_watchers: Rc::new(RefCell::new(vec![])),
            config_head: RefCell::new(config_head),
            profile_heads: RefCell::new(profile_heads),
            chats: RefCell::new(chats),
            messages_watchers: Rc::new(RefCell::new(vec![])),
        })
    }

    pub fn set_audio_interface(&self, interface: Option<Box<dyn AudioInterface>>) {
        *self.audio_interface.borrow_mut() = interface;
    }

    fn ensure_own_profile_head(&self) {
        let head_name = user_path(&self.public_key());
        let contact_keys = self
            .contacts_snapshot()
            .values()
            .filter(|c| c.contact_type != ContactType::Blocked)
            .map(|c| c.public_key.clone())
            .collect::<Vec<_>>();
        let mut profiles = self.profile_heads.borrow_mut();
        let head = profiles
            .entry(head_name.clone())
            .or_insert_with(|| signed_store_head(head_name, &self.store_dir));
        for key in contact_keys {
            head.remote_add(key, None);
        }
    }

    fn ensure_profile_head(&self, public_key: &str) {
        let head_name = user_path(public_key);
        self.profile_heads
            .borrow_mut()
            .entry(head_name.clone())
            .or_insert_with(|| signed_store_head(head_name, &self.store_dir));
    }

    fn ensure_chat_head(&self, chat_id: &str) {
        let peer_key = peer_key_from_chat_path(&self.public_key(), chat_id);
        self.chats
            .borrow_mut()
            .entry(chat_id.to_string())
            .or_insert_with(|| {
                let mut head = signed_store_head(chat_id.to_string(), &self.store_dir);
                if let Some(peer_key) = peer_key {
                    head.remote_add(peer_key, None);
                }
                head
            });
    }

    fn contacts_snapshot(&self) -> HashMap<String, Contact> {
        let contacts_head = self.contacts_head.borrow();
        contacts_from_head(&contacts_head)
    }

    fn chat_messages_snapshot(&self, chat_id: &str) -> Option<HashMap<String, ChatMessage>> {
        let chats = self.chats.borrow();
        let head = chats.get(chat_id)?;
        let our_key = self.public_key();
        let peer_key = peer_key_from_chat_path(&our_key, chat_id)?;
        let messages = chat_messages_from_head(head, &our_key, &peer_key);
        if messages.is_empty() {
            None
        } else {
            Some(messages)
        }
    }

    fn chat_ids(&self) -> Vec<String> {
        self.chats.borrow().keys().cloned().collect()
    }

    fn signed_record(&self, record: HoshiRecord) -> Result<HoshiSignedRecord> {
        HoshiSignedRecord::sign(record, &self.identity)
    }

    fn sync_head(&self, head_name: &str, head: &mut StoreHead<HoshiSignedRecord>) {
        let from_key = self.public_key();
        let head_name = head_name.to_string();
        head.tx(|dest, cmd| {
            self.net.send(HoshiMessage::new(
                from_key.clone(),
                dest,
                HoshiNetPayload::StoreSync {
                    head_name: head_name.clone(),
                    command: cmd,
                },
            ));
        });
    }

    fn drain_pending_heads(&self) -> StepChanges {
        let mut changes = StepChanges::default();

        if self.contacts_head.borrow_mut().step() > 0 {
            changes.note_contacts();
        }
        let _ = self.config_head.borrow_mut().step();
        {
            let mut profiles = self.profile_heads.borrow_mut();
            for head in profiles.values_mut() {
                if head.step() > 0 {
                    changes.note_profiles();
                }
            }
        }
        {
            let mut chats = self.chats.borrow_mut();
            for (chat_id, head) in chats.iter_mut() {
                if head.step() > 0 {
                    changes.note_chat(chat_id.clone());
                }
            }
        }

        changes
    }

    fn sync_store_heads(&self) {
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
    }

    fn notify_contacts_watchers(&self) {
        let watchers = self
            .contacts_watchers
            .borrow()
            .iter()
            .map(|watcher| Rc::clone(&watcher.fun))
            .collect::<Vec<_>>();
        if watchers.is_empty() {
            return;
        }

        let contacts = self.contacts_snapshot();
        for watcher in watchers {
            watcher(self, &contacts);
        }
    }

    fn notify_messages_watchers(&self, chat_id: &str) {
        let watchers = self
            .messages_watchers
            .borrow()
            .iter()
            .filter(|watcher| {
                watcher
                    .filter
                    .as_deref()
                    .is_none_or(|filter| filter == chat_id)
            })
            .map(|watcher| Rc::clone(&watcher.fun))
            .collect::<Vec<_>>();
        if watchers.is_empty() {
            return;
        }

        let Some(messages) = self.chat_messages_snapshot(chat_id) else {
            return;
        };
        for watcher in watchers {
            watcher(self, chat_id, &messages);
        }
    }

    fn notify_all_message_watchers(&self) {
        for chat_id in self.chat_ids() {
            self.notify_messages_watchers(&chat_id);
        }
    }

    fn notify_calls_watchers(&self) {
        let watchers = self
            .calls_watchers
            .borrow()
            .iter()
            .map(|watcher| Rc::clone(&watcher.fun))
            .collect::<Vec<_>>();
        if watchers.is_empty() {
            return;
        }

        let calls = self.calls.borrow().clone();
        for watcher in watchers {
            watcher(self, &calls);
        }
    }

    fn process_store_sync(
        &self,
        from_key: &str,
        head_name: String,
        command: HeadCommand<HoshiSignedRecord>,
        changes: &mut StepChanges,
    ) {
        if head_name.starts_with("/chat/") {
            self.process_chat_sync(from_key, head_name, command, changes);
        } else if head_name.starts_with("/user/") {
            self.process_profile_sync(from_key, head_name, command, changes);
        }
    }

    fn process_chat_sync(
        &self,
        from_key: &str,
        head_name: String,
        command: HeadCommand<HoshiSignedRecord>,
        changes: &mut StepChanges,
    ) {
        let Some(peer_key) = self.peer_key_for_chat_path(&head_name) else {
            return;
        };
        if peer_key != from_key {
            return;
        }
        if let HeadCommand::Put(record) = &command
            && (!record.verify()
                || (record.record.from != self.public_key() && record.record.from != peer_key))
        {
            return;
        }
        self.ensure_chat_head(&head_name);
        if self.contact_get(from_key).is_none() {
            let _ = self.contact_upsert(Contact::new_unknown(from_key.to_string()));
        }
        let mut chats = self.chats.borrow_mut();
        if let Some(head) = chats.get_mut(&head_name)
            && head.rx(from_key, command)
        {
            changes.note_chat(head_name);
        }
    }

    fn process_profile_sync(
        &self,
        from_key: &str,
        head_name: String,
        command: HeadCommand<HoshiSignedRecord>,
        changes: &mut StepChanges,
    ) {
        let own_key = self.public_key();
        let is_owner_sync = head_name == user_path(from_key);
        let is_request_for_own_profile = head_name == user_path(&own_key)
            && matches!(&command, HeadCommand::Tip(_) | HeadCommand::TipSecondary(_));
        if !is_owner_sync && !is_request_for_own_profile {
            return;
        }
        if let HeadCommand::Put(record) = &command
            && (!record.verify() || record.record.from != from_key)
        {
            return;
        }
        if is_owner_sync {
            self.ensure_profile_head(from_key);
        } else {
            self.ensure_profile_head(&own_key);
        }
        let mut profiles = self.profile_heads.borrow_mut();
        if let Some(head) = profiles.get_mut(&head_name)
            && head.rx(from_key, command)
        {
            changes.note_profiles();
        }
    }

    fn process_net_message(&self, net_msg: HoshiMessage, changes: &mut StepChanges) {
        match net_msg.payload {
            HoshiNetPayload::UpdateCallState { call_id, events } => {
                if self.contact_get(&net_msg.from_key).is_none() {
                    let _ = self.contact_upsert(Contact::new_unknown(net_msg.from_key.clone()));
                }
                let mut found = false;
                let contact_lookup = |key: &str| -> Contact { self.contact_for(key) };
                for call in self.calls.borrow_mut().iter_mut() {
                    if call.id() == &call_id {
                        call.merge_events(events.clone(), &contact_lookup);
                        self.close_call_if_remote_ended(call);
                        found = true;
                        break;
                    }
                }
                if !found {
                    let mut call = Call::from_events(call_id, events, &contact_lookup);
                    let own_status = call.get_status(&self.public_key());
                    match own_status {
                        Some(CallPartyStatus::Invited) => {
                            call.add_event(CallPartyEvent::new(
                                self.public_key(),
                                CallPartyStatus::Ringing,
                            ));
                            let msgs = self.build_call_state_messages(&call);
                            self.attach_audio(&call);
                            self.calls.borrow_mut().push(call);
                            for msg in msgs {
                                self.net.send(msg);
                            }
                        }
                        Some(CallPartyStatus::Ringing | CallPartyStatus::Active) => {
                            self.attach_audio(&call);
                            self.calls.borrow_mut().push(call);
                        }
                        Some(CallPartyStatus::HungUp) | None => {}
                    }
                }
                self.notify_calls_watchers();
            }
            HoshiNetPayload::AudioChunk { call_id, chunk } => {
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
            HoshiNetPayload::StoreSync { head_name, command } => {
                self.process_store_sync(&net_msg.from_key, head_name, command, changes);
            }
            _ => {}
        }
    }

    fn step_calls(&self) -> usize {
        let before = self.calls.borrow().len();
        let own_key = self.public_key();
        let mut stepped = 0;
        self.calls.borrow_mut().retain_mut(|call| {
            stepped += 1;
            call.step(self);

            let own_status = call.get_status(&own_key);
            match own_status {
                None | Some(CallPartyStatus::HungUp) => false,
                Some(_) => {
                    let has_other_party = call
                        .non_hungup_party_keys()
                        .into_iter()
                        .any(|key| key != own_key);
                    if has_other_party {
                        call.call_ended = None;
                        true
                    } else if call.all_other_parties_hung_up(&own_key) {
                        false
                    } else {
                        let ended_at = call.call_ended.get_or_insert_with(std::time::Instant::now);
                        ended_at.elapsed().as_secs() < CALL_END_GRACE_SECS
                    }
                }
            }
        });
        if self.calls.borrow().len() != before {
            self.notify_calls_watchers();
        }
        stepped
    }

    /// Derive the peer key for a chat path from the XOR hash
    pub fn peer_key_for_chat_path(&self, cp: &str) -> Option<String> {
        peer_key_from_chat_path(&self.public_key(), cp)
    }

    pub fn call_start(&self, parties: Vec<Contact>) {
        let call = Call::new(self.public_key(), parties);

        self.attach_audio(&call);
        self.send_call_state(&call);
        self.calls.borrow_mut().push(call);
        self.notify_calls_watchers();
    }

    pub fn calls_watch<F>(&self, f: F) -> HoshiWatchRef
    where
        F: Fn(&Self, &[Call]) + 'static,
    {
        let fun: CallWatchFn = Rc::new(f);
        let calls = self.calls.borrow().clone();
        fun(self, calls.as_slice());
        register_watch(&self.calls_watchers, fun)
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

    fn attach_audio(&self, call: &Call) {
        if let Some(interface) = self.audio_interface.borrow().as_ref()
            && let Ok(stream) = interface.create(self, call)
        {
            call.set_audio(Some(stream));
        }
    }

    pub(crate) fn send_call_state(&self, call: &Call) {
        for msg in self.build_call_state_messages(call) {
            self.net.send(msg);
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
        self.notify_calls_watchers();
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
        self.notify_calls_watchers();
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
        self.notify_calls_watchers();
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
                    HoshiNetPayload::UpdateCallState {
                        call_id: call.id().to_string(),
                        events: call.events().to_vec(),
                    },
                )
            })
            .collect()
    }

    fn close_call_if_remote_ended(&self, call: &mut Call) {
        let my_key = self.public_key();
        let own_status = call.get_status(&my_key);
        if matches!(own_status, None | Some(CallPartyStatus::HungUp)) {
            return;
        }
        if !call.all_other_parties_hung_up(&my_key) {
            return;
        }

        call.add_event(CallPartyEvent::new(my_key.clone(), CallPartyStatus::HungUp));
        for msg in self.build_call_state_messages(call) {
            self.net.send(msg);
        }
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
            && alias_from_head(head, &self.public_key()) == Some(alias.to_string())
        {
            return;
        }
        drop(profiles);
        let mut profiles = self.profile_heads.borrow_mut();
        if let Some(head) = profiles.get_mut(&head_name) {
            let record = HoshiRecord::new(
                self.public_key(),
                HoshiPayload::Title {
                    title: alias.to_string(),
                },
            );
            if let Ok(record) = self.signed_record(record) {
                head.queue(record);
            }
        }
    }

    pub fn user_alias(&self, public_key: &str) -> Option<String> {
        let head_name = user_path(public_key);
        self.profile_heads
            .borrow()
            .get(&head_name)
            .and_then(|head| alias_from_head(head, public_key))
    }

    pub fn display_name(&self, public_key: &str) -> String {
        self.user_alias(public_key)
            .unwrap_or_else(|| self.contact_for(public_key).display_name())
    }

    pub fn set_public_key(&self, key: String) -> Result<()> {
        let key = normalize_public_key(&key);
        validate_public_key_hex(&key)?;
        if key != self.identity.public_key_hex() {
            return Err(anyhow!(
                "public key must match the loaded identity public key"
            ));
        }
        self.net.disconnect_all();
        let record = self.signed_record(HoshiRecord::new(
            self.public_key(),
            HoshiPayload::Config {
                key: CONFIG_KEY_PUBLIC_KEY.to_string(),
                value: key.clone(),
            },
        ))?;
        self.config_head.borrow_mut().insert(record);
        *self.public_key.borrow_mut() = key;
        self.ensure_own_profile_head();
        self.notify_contacts_watchers();
        self.notify_all_message_watchers();
        Ok(())
    }

    pub fn last_message(&self, chat_id: &str) -> Option<ChatMessage> {
        self.chat_messages_snapshot(chat_id)?
            .values()
            .max()
            .cloned()
    }

    /// Main function a client MUST call regularly for the clientlib
    /// to work as expected. It communicates with the various other
    /// threads and updates the internal state as well as fires callbacks.
    pub fn step(&self) -> u32 {
        let mut changes = self.drain_pending_heads();
        let mut msgs = 0;
        for net_msg in self.net.step() {
            msgs += 1;
            self.process_net_message(net_msg, &mut changes);
        }

        changes.merge(self.drain_pending_heads());
        self.sync_store_heads();
        msgs += self.step_calls() as u32;

        if changes.has_contact_view_changes() {
            self.notify_contacts_watchers();
        }
        for chat_id in changes.changed_chat_ids.iter() {
            self.notify_messages_watchers(chat_id);
        }

        msgs
    }

    pub fn message_upsert(&self, msg: ChatMessage) -> Result<()> {
        if msg.from != self.public_key() {
            return Err(anyhow!("message sender must match client public key"));
        }
        let chat_id = msg.chat_id();
        self.ensure_chat_head(&chat_id);
        if let Some(peer_key) = self.peer_key_for_chat_path(&chat_id) {
            let mut chats = self.chats.borrow_mut();
            if let Some(head) = chats.get_mut(&chat_id) {
                head.remote_add(peer_key, None);
                let id = Uuid::parse_str(&msg.id).unwrap_or_else(|_| Uuid::now_v7());
                let record = self.signed_record(HoshiRecord::with_id(
                    id,
                    msg.from.clone(),
                    HoshiPayload::Text {
                        content: msg.content.clone(),
                    },
                ))?;
                head.queue(record);
            }
        }
        Ok(())
    }

    /// Call this function to get notified whenever messages in a
    /// particular chat_id as specified by filter changes. Use None to
    /// get notified about all messages.
    #[inline]
    pub fn messages_watch<F>(&self, filter: Option<String>, f: F) -> HoshiWatchRef
    where
        F: Fn(&HoshiClient, &str, &HashMap<String, ChatMessage>) + 'static,
    {
        let fun: MessageWatchFn = Rc::new(f);
        match filter.as_deref() {
            None => {
                for chat_id in self.chat_ids() {
                    if let Some(chat) = self.chat_messages_snapshot(&chat_id) {
                        fun(self, &chat_id, &chat);
                    }
                }
            }
            Some(chat_id) => {
                if let Some(chat) = self.chat_messages_snapshot(chat_id) {
                    fun(self, chat_id, &chat);
                }
            }
        }

        let id = Uuid::now_v7();
        self.messages_watchers.borrow_mut().push(MessageWatcher {
            id,
            filter,
            fun: Rc::clone(&fun),
        });
        let watchers = Rc::clone(&self.messages_watchers);
        HoshiWatchRef::new(move || {
            watchers.borrow_mut().retain(|watcher| watcher.id != id);
        })
    }

    /// Call f with a current snapshot of the current contacts once
    #[inline]
    pub fn with_contacts<F>(&self, f: F)
    where
        F: FnOnce(&Self, &HashMap<String, Contact>),
    {
        let contacts = self.contacts_snapshot();
        f(self, &contacts);
    }

    /// Use this function so that f gets called whenever a contact changes.
    #[inline]
    pub fn contacts_watch<F>(&self, f: F) -> HoshiWatchRef
    where
        F: Fn(&HoshiClient, &HashMap<String, Contact>) + 'static,
    {
        let fun: ContactWatchFn = Rc::new(f);
        let contacts = self.contacts_snapshot();
        fun(self, &contacts);
        register_watch(&self.contacts_watchers, fun)
    }

    /// Lookup a particular public_key in the current snapshot of Contacts
    #[inline]
    pub fn contact_get(&self, public_key: &str) -> Option<Contact> {
        self.contacts_snapshot().get(public_key).cloned()
    }

    /// Update or Insert a particular Contact, currently only persists locally.
    pub fn contact_upsert(&self, mut contact: Contact) -> Result<()> {
        contact.public_key = normalize_public_key(&contact.public_key);
        validate_public_key_hex(&contact.public_key)?;
        let record = self.signed_record(HoshiRecord::new(
            self.public_key(),
            HoshiPayload::Contact {
                public_key: contact.public_key.clone(),
                contact_type: contact.contact_type.clone(),
            },
        ))?;
        self.contacts_head.borrow().queue(record);

        configure_contact_heads(
            &self.public_key(),
            &self.store_dir,
            &mut self.chats.borrow_mut(),
            &mut self.profile_heads.borrow_mut(),
            &contact,
            true,
        );

        Ok(())
    }

    /// Remove a contact locally.
    pub fn contact_delete(&self, public_key: &str) -> Result<()> {
        let record = self.signed_record(HoshiRecord::new(
            self.public_key(),
            HoshiPayload::Contact {
                public_key: public_key.to_string(),
                contact_type: ContactType::Deleted,
            },
        ))?;
        self.contacts_head.borrow().queue(record);
        let own_path = user_path(&self.public_key());
        let mut profiles = self.profile_heads.borrow_mut();
        if let Some(head) = profiles.get_mut(&own_path) {
            head.remote_drop(public_key);
        }
        if let Some(head) = profiles.get_mut(&user_path(public_key)) {
            head.remote_drop(public_key);
        }

        Ok(())
    }
}

impl std::fmt::Debug for HoshiClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HoshiClient")
            .field("contacts", &self.contacts_snapshot().len())
            .field(
                "contacts_watchers",
                &format!("[{} watchers]", self.contacts_watchers.borrow().len()),
            )
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn added_contact_can_request_and_receive_profile_immediately() -> Result<()> {
        let dir_a = tempfile::tempdir()?;
        let dir_b = tempfile::tempdir()?;
        let client_a = HoshiClient::new(Some(dir_a.path().to_path_buf()))?;
        let client_b = HoshiClient::new(Some(dir_b.path().to_path_buf()))?;
        let key_a = client_a.public_key();
        let key_b = client_b.public_key();

        client_b.set_user_alias("Bob");
        client_b.step();

        client_a.contact_upsert(Contact::new(key_b.clone()))?;

        let profile_requests = {
            let mut profiles = client_a.profile_heads.borrow_mut();
            let head = profiles
                .get_mut(&user_path(&key_b))
                .expect("contact profile head should exist");
            let mut commands = Vec::new();
            head.tx(|dest, command| commands.push((dest, command)));
            commands
        };
        assert!(
            profile_requests
                .iter()
                .any(|(dest, command)| dest == &key_b && matches!(command, HeadCommand::Tip(_))),
            "adding a contact should queue an initial profile tip request"
        );

        let mut changes = StepChanges::default();
        for (_, command) in profile_requests {
            client_b.process_store_sync(&key_a, user_path(&key_b), command, &mut changes);
        }

        let profile_responses = {
            let mut profiles = client_b.profile_heads.borrow_mut();
            let head = profiles
                .get_mut(&user_path(&key_b))
                .expect("own profile head should exist");
            let mut commands = Vec::new();
            head.tx(|dest, command| commands.push((dest, command)));
            commands
        };
        assert!(
            profile_responses.iter().any(|(dest, command)| {
                dest == &key_a
                    && matches!(
                        command,
                        HeadCommand::Put(record)
                            if matches!(record.record.payload, HoshiPayload::Title { .. })
                    )
            }),
            "profile owner should push profile records after a tip request"
        );

        let mut changes = StepChanges::default();
        for (_, command) in profile_responses {
            client_a.process_store_sync(&key_b, user_path(&key_b), command, &mut changes);
        }

        assert_eq!(client_a.display_name(&key_b), "Bob");
        Ok(())
    }

    #[test]
    fn blocked_contact_does_not_request_profile_tip() -> Result<()> {
        let dir_a = tempfile::tempdir()?;
        let dir_b = tempfile::tempdir()?;
        let client_a = HoshiClient::new(Some(dir_a.path().to_path_buf()))?;
        let client_b = HoshiClient::new(Some(dir_b.path().to_path_buf()))?;
        let key_b = client_b.public_key();

        let mut contact = Contact::new(key_b.clone());
        contact.contact_type = ContactType::Blocked;
        client_a.contact_upsert(contact)?;

        let profile_requests = {
            let mut profiles = client_a.profile_heads.borrow_mut();
            let head = profiles
                .get_mut(&user_path(&key_b))
                .expect("blocked contact profile head should exist");
            let mut commands = Vec::new();
            head.tx(|dest, command| commands.push((dest, command)));
            commands
        };

        assert!(
            profile_requests.is_empty(),
            "blocked contacts should not queue profile sync requests"
        );
        Ok(())
    }
}
