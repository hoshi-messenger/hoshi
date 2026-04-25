use std::{cell::RefCell, collections::HashSet, f32::consts::PI, ops::Rem, rc::Rc, time::Instant};

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{
    AudioStream, Contact,
    audio_chunk::{AudioChunk, linear_to_ulaw},
    hoshi_client::HoshiClient,
    hoshi_net_client::{HoshiMessage, HoshiNetPayload},
};

#[derive(Clone, Debug)]
pub struct Call {
    id: String,
    parties: Vec<CallParty>,
    events: Vec<CallPartyEvent>,

    pub(crate) audio: Rc<RefCell<Option<Box<dyn AudioStream>>>>,

    ring_samples_written: usize,
    audio_send_start: Option<Instant>,
    audio_samples_sent: usize,
    last_invite: Option<Instant>,
    pub call_started: Instant,
    pub call_ended: Option<Instant>,
}

fn dtmf_freqs(digit: char) -> Option<(f32, f32)> {
    match digit {
        // Standard digits
        '1' => Some((697.0, 1209.0)),
        '2' => Some((697.0, 1336.0)),
        '3' => Some((697.0, 1477.0)),
        '4' => Some((770.0, 1209.0)),
        '5' => Some((770.0, 1336.0)),
        '6' => Some((770.0, 1477.0)),
        '7' => Some((852.0, 1209.0)),
        '8' => Some((852.0, 1336.0)),
        '9' => Some((852.0, 1477.0)),
        '0' => Some((941.0, 1336.0)),
        // Military DTMF (4th column, 1633Hz)
        'A' | 'a' => Some((697.0, 1633.0)),
        'B' | 'b' => Some((770.0, 1633.0)),
        'C' | 'c' => Some((852.0, 1633.0)),
        'D' | 'd' => Some((941.0, 1633.0)),
        // E/F mapped to */#
        'E' | 'e' | '*' => Some((941.0, 1209.0)), // *
        'F' | 'f' | '#' => Some((941.0, 1477.0)), // #
        _ => None,
    }
}

impl Call {
    pub fn new(own_key: String, parties: Vec<Contact>) -> Self {
        let mut events = Vec::new();
        events.push(CallPartyEvent::new(
            own_key.clone(),
            CallPartyStatus::Active,
        ));
        for p in &parties {
            events.push(CallPartyEvent::new(
                p.public_key.clone(),
                CallPartyStatus::Invited,
            ));
        }

        let mut call_parties: Vec<CallParty> = parties.into_iter().map(|p| p.into()).collect();
        call_parties.push(CallParty {
            contact: Contact::new(own_key),
            status: CallPartyStatus::Active,
        });

        Self {
            id: uuid::Uuid::now_v7().to_string(),
            parties: call_parties,
            events,
            audio: Rc::new(RefCell::new(None)),
            ring_samples_written: 0,
            audio_send_start: None,
            audio_samples_sent: 0,
            last_invite: None,
            call_started: Instant::now(),
            call_ended: None,
        }
    }

    pub fn from_events(
        id: String,
        events: Vec<CallPartyEvent>,
        contact_lookup: impl Fn(&str) -> Contact,
    ) -> Self {
        let mut call = Self {
            id,
            parties: vec![],
            events: vec![],
            audio: Rc::new(RefCell::new(None)),
            audio_send_start: None,
            audio_samples_sent: 0,
            last_invite: None,
            call_started: Instant::now(),
            ring_samples_written: 0,
            call_ended: None,
        };
        call.merge_events(events, &contact_lookup);
        call
    }

    pub fn set_audio(&self, sink: Option<Box<dyn AudioStream>>) {
        *self.audio.borrow_mut() = sink;
    }

    pub fn active_party_count(&self) -> usize {
        self.parties
            .iter()
            .filter(|p| matches!(p.status, CallPartyStatus::Active))
            .count()
    }

    pub fn active_or_ringing_party_count(&self) -> usize {
        self.parties
            .iter()
            .filter(|p| !matches!(p.status, CallPartyStatus::HungUp))
            .count()
    }

    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn get_status(&self, public_key: &str) -> Option<CallPartyStatus> {
        self.parties
            .iter()
            .find(|p| p.contact.public_key == public_key)
            .map(|p| p.status)
    }

    pub fn events(&self) -> &[CallPartyEvent] {
        &self.events
    }

    pub fn add_event(&mut self, event: CallPartyEvent) {
        self.events.push(event);
        self.rebuild_parties(&|key| Contact::new(key.to_string()));
    }

    pub fn add_event_with_contact(&mut self, event: CallPartyEvent, contact: Contact) {
        // Ensure the contact is in parties before rebuilding so alias is preserved
        if !self
            .parties
            .iter()
            .any(|p| p.contact.public_key == contact.public_key)
        {
            self.parties.push(CallParty {
                status: event.status(),
                contact,
            });
        }
        self.events.push(event);
        self.rebuild_parties(&|key| Contact::new(key.to_string()));
    }

    pub fn merge_events(
        &mut self,
        incoming: Vec<CallPartyEvent>,
        contact_lookup: &impl Fn(&str) -> Contact,
    ) {
        let existing_ids: HashSet<Uuid> = self.events.iter().map(|e| e.id).collect();
        for event in incoming {
            if !existing_ids.contains(&event.id) {
                self.events.push(event);
            }
        }
        self.rebuild_parties(contact_lookup);
    }

    fn rebuild_parties(&mut self, contact_lookup: &impl Fn(&str) -> Contact) {
        self.events.sort();
        let mut status_map: Vec<(&str, CallPartyStatus)> = Vec::new();
        for event in &self.events {
            if let Some(entry) = status_map.iter_mut().find(|(k, _)| *k == event.key()) {
                entry.1 = event.status();
            } else {
                status_map.push((event.key(), event.status()));
            }
        }
        for (key, status) in &status_map {
            if let Some(party) = self
                .parties
                .iter_mut()
                .find(|p| p.contact.public_key == *key)
            {
                party.status = *status;
            } else {
                self.parties.push(CallParty {
                    contact: contact_lookup(key),
                    status: *status,
                });
            }
        }
    }

    pub fn non_hungup_party_keys(&self) -> Vec<String> {
        self.parties
            .iter()
            .filter(|p| !matches!(p.status, CallPartyStatus::HungUp))
            .map(|p| p.contact.public_key.clone())
            .collect()
    }

    pub fn all_other_parties_hung_up(&self, own_key: &str) -> bool {
        let mut saw_other_party = false;
        for party in &self.parties {
            if party.contact.public_key == own_key {
                continue;
            }
            saw_other_party = true;
            if !matches!(party.status, CallPartyStatus::HungUp) {
                return false;
            }
        }
        saw_other_party
    }

    pub fn get_call_label<F>(&self, own_contact: Contact, display_name: F) -> String
    where
        F: Fn(&str) -> String,
    {
        let other_parties: Vec<&CallParty> = self
            .parties
            .iter()
            .filter(|p| p.contact.public_key != own_contact.public_key)
            .collect();

        let names: Vec<String> = other_parties
            .iter()
            .map(|p| match p.status {
                CallPartyStatus::Invited => {
                    format!("{} (dialing)", display_name(&p.contact.public_key))
                }
                CallPartyStatus::Ringing => {
                    format!("{} (ringing)", display_name(&p.contact.public_key))
                }
                _ => display_name(&p.contact.public_key),
            })
            .collect();

        if let Some(status) = self.get_status(&own_contact.public_key) {
            match status {
                CallPartyStatus::Active => {
                    if self.active_party_count() > 1 {
                        let s = self.call_started.elapsed().as_secs();
                        let m = s / 60;
                        let s = s % 60;
                        format!("Call {} - {:02}:{:02}", names.join(", "), m, s)
                    } else {
                        format!("Calling {}", names.join(", "))
                    }
                }
                CallPartyStatus::HungUp => {
                    format!("Call ended with: {}", names.join(", "))
                }
                CallPartyStatus::Ringing | CallPartyStatus::Invited => {
                    format!("Incoming call from {}", names.join(", "))
                }
            }
        } else {
            format!("Call ended with: {}", names.join(", "))
        }
    }

    pub fn get_party_public_keys(&self) -> Vec<String> {
        self.parties
            .iter()
            .map(|p| p.contact.public_key.clone())
            .collect()
    }

    pub fn get_party_index(&self, public_key: &str) -> Option<usize> {
        self.parties
            .iter()
            .position(|p| p.contact.public_key == public_key)
    }

    pub fn get_parties(&self) -> Vec<CallParty> {
        self.parties.clone()
    }

    /// Receives an incoming audio chunk, decodes it and writes it to the audio sink.
    pub fn receive_audio(&mut self, chunk: AudioChunk, from_key: &str) {
        if let Some(party_index) = self.get_party_index(from_key) {
            if let Some(sink) = self.audio.borrow().as_ref() {
                let samples = chunk.decode_i16(); // i16 samples at 24kHz
                let samples = match chunk.sample_rate() {
                    24000 => {
                        // Upsample 24kHz → 48kHz by repeating each sample
                        samples.iter().flat_map(|&s| [s, s]).collect()
                    }
                    48000 => samples,
                    _ => {
                        eprintln!("Invalid sample_rate {}", chunk.sample_rate());
                        samples
                    }
                };
                sink.write(party_index + 1, &samples);
            }
        }
    }

    pub fn step(&mut self, client: &HoshiClient) {
        // --- Periodic re-send of full call state to all non-HungUp parties ---
        let now = Instant::now();
        let should_send = self
            .last_invite
            .map_or(true, |t| now.duration_since(t).as_secs() >= 1);

        if should_send {
            self.last_invite = Some(now);
            client.send_call_state(self);
        }

        // --- Audio: recover from stream errors ---
        if self.audio.borrow().as_ref().is_some_and(|s| s.has_error()) {
            eprintln!("[audio] stream error detected, recreating audio stream");
            self.set_audio(None);
            self.audio_send_start = None;
            self.audio_samples_sent = 0;
            if let Some(interface) = client.audio_interface.borrow().as_ref() {
                if let Ok(stream) = interface.create(client, self) {
                    self.set_audio(Some(stream));
                }
            }
        }

        // --- Audio: start/stop sink and source based on party status ---
        if self.parties.len() > 1 {
            self.audio.borrow().as_ref().map(|s| s.play());
        } else {
            self.audio.borrow().as_ref().map(|s| s.pause());
        }

        // --- Capture and send audio: drain all available source data ---
        let my_key = client.public_key();
        let my_status = self.get_status(&my_key);
        match my_status {
            Some(CallPartyStatus::Active) => (),
            Some(CallPartyStatus::Ringing) => {
                let time_elapsed = self.call_started.elapsed().as_millis();
                let samples_required = ((time_elapsed + 80) * (48000 / 1000)) as usize;
                let samples_required = ((samples_required / 960) + 1) * 960;
                if samples_required > self.ring_samples_written {
                    let mut samples_48k = [0i16; 2048];
                    samples_48k.iter_mut().enumerate().for_each(|(i, s)| {
                        let i = i + self.ring_samples_written;
                        let t = (i as f32) * (1.0 / 48000.0);

                        let digit_i = (t * (1000.0 / 48.0)) as usize;
                        if digit_i & 16 == 0 {
                            let a = (t * 440.0 * 2.0 * PI).sin();
                            let envelope = (t * 20.0 * 2.0 * PI).sin() * 0.5 + 0.5;
                            let a = a * envelope * 0.8;
                            *s = (a * 32760.0) as i16;
                        };
                    });
                    self.ring_samples_written += 2048;
                    if let Some(sink) = self.audio.borrow().as_ref() {
                        sink.write(0, &samples_48k);
                    }
                };
                return;
            }
            _ => return,
        }

        let active_keys: Vec<String> = self
            .parties
            .iter()
            .filter(|p| {
                matches!(p.status, CallPartyStatus::Active) && p.contact.public_key != my_key
            })
            .map(|p| p.contact.public_key.clone())
            .collect();
        if active_keys.len() == 0 {
            let public_key = self
                .parties
                .iter()
                .find(|c| {
                    matches!(
                        c.status,
                        CallPartyStatus::Ringing | CallPartyStatus::Invited
                    )
                })
                .map(|c| c.contact.public_key.as_str())
                .unwrap_or_default();
            let time_elapsed = self.call_started.elapsed().as_millis();
            let samples_required = ((time_elapsed + 80) * (48000 / 1000)) as usize;
            let samples_required = ((samples_required / 960) + 1) * 960;
            if samples_required > self.ring_samples_written {
                let mut samples_48k = [0i16; 2048];
                samples_48k.iter_mut().enumerate().for_each(|(i, s)| {
                    let i = i + self.ring_samples_written;
                    let t = (i as f32) * (1.0 / 48000.0);

                    // DTMF Dialing - 32ms on - 16ms off
                    let digit_i = t * (1000.0 / 48.0);
                    let digit_phase = digit_i.rem(3.0);
                    let digit_i = (digit_i as usize) / 3;
                    if digit_i < 8
                        && let Some(digit) = public_key.chars().nth(digit_i)
                        && digit_phase < 2.0
                    {
                        let dtmf = dtmf_freqs(digit).unwrap_or_default();
                        let a = (t * dtmf.0 * 2.0 * PI).sin();
                        let b = (t * dtmf.1 * 2.0 * PI).sin();
                        let a = (a + b) * 0.1;
                        *s = (a * 32760.0) as i16;
                    }
                    if digit_i > 8 {
                        if (digit_i & 16) == 0 {
                            let a = (t * 440.0 * 2.0 * PI).sin();
                            let b = (t * 480.0 * 2.0 * PI).sin();
                            let a = (a + b) * 0.1;
                            *s = (a * 32760.0) as i16;
                        }
                    }
                });
                self.ring_samples_written += 2048;
                if let Some(sink) = self.audio.borrow().as_ref() {
                    sink.write(0, &samples_48k);
                }
            }
            return;
        }

        // Drain available audio from the source buffer in 1024-sample chunks,
        // capped by a budget based on wall-clock time to avoid flooding the network.
        let send_start = *self.audio_send_start.get_or_insert_with(Instant::now);
        let elapsed_samples = (send_start.elapsed().as_secs_f64() * 48000.0) as usize;
        // Allow sending up to 64k samples ahead of real-time to absorb stalls.
        let max_samples = elapsed_samples + 8192;

        loop {
            if self.audio_samples_sent >= max_samples {
                break;
            }

            let mut samples_48k = [0i16; 1024];
            let samples_read = if let Some(source) = self.audio.borrow().as_ref() {
                let n = source.read(&mut samples_48k);
                if n == 0 {
                    break;
                }
                n
            } else {
                break;
            };

            self.audio_samples_sent += samples_read;

            // Encode as μ-law
            let encoded: Vec<u8> = samples_48k[..samples_read]
                .iter()
                .map(|&s| linear_to_ulaw(s))
                .collect();

            let chunk = AudioChunk::ULaw {
                sample_rate: 48_000,
                samples: encoded,
            };
            for key in &active_keys {
                client.net.send(HoshiMessage::new(
                    my_key.clone(),
                    key.clone(),
                    HoshiNetPayload::AudioChunk {
                        call_id: self.id.clone(),
                        chunk: chunk.clone(),
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

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum CallPartyStatus {
    Invited,
    Ringing,
    HungUp,
    Active,
}

impl From<Contact> for CallParty {
    fn from(value: Contact) -> Self {
        Self {
            contact: value,
            status: CallPartyStatus::Invited,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct CallPartyEvent {
    id: Uuid,
    key: String,
    status: CallPartyStatus,
}

impl CallPartyEvent {
    pub fn new(key: String, status: CallPartyStatus) -> Self {
        Self {
            id: Uuid::now_v7(),
            key,
            status,
        }
    }

    pub fn key(&self) -> &str {
        &self.key
    }

    pub fn status(&self) -> CallPartyStatus {
        self.status
    }
}

impl PartialOrd for CallPartyEvent {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for CallPartyEvent {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.id.cmp(&other.id)
    }
}
