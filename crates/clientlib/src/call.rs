use std::{cell::RefCell, f32::consts::PI, ops::Rem, rc::Rc, time::Instant};

use serde::{Deserialize, Serialize};

use crate::{
    AudioStream, Contact,
    audio_chunk::{AudioChunk, linear_to_ulaw},
    hoshi_client::HoshiClient,
    hoshi_net_client::{HoshiMessage, HoshiPayload},
};

#[derive(Clone, Debug)]
pub struct Call {
    id: String,
    parties: Vec<CallParty>,

    pub(crate) audio: Rc<RefCell<Option<Box<dyn AudioStream>>>>,

    ring_samples_written: usize,
    last_audio_send: Option<Instant>,
    last_invite: Option<Instant>,
    pub call_started: Instant,
    pub call_ended: Option<Instant>,
    last_ring: Option<Instant>,
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
    pub fn new(parties: Vec<Contact>) -> Self {
        let parties = parties.into_iter().map(|p| p.into()).collect();
        Self {
            id: uuid::Uuid::now_v7().to_string(),
            parties,
            audio: Rc::new(RefCell::new(None)),
            ring_samples_written: 0,
            last_invite: None,
            call_started: Instant::now(),
            call_ended: None,
            last_ring: None,
            last_audio_send: None,
        }
    }

    pub fn from_invite(id: String, caller: Contact, own_contact: Contact) -> Self {
        let mut caller: CallParty = caller.into();
        caller.status = CallPartyStatus::Active;

        Self {
            id,
            parties: vec![caller, own_contact.into()],
            audio: Rc::new(RefCell::new(None)),
            last_invite: None,
            call_started: Instant::now(),
            ring_samples_written: 0,
            call_ended: None,
            last_ring: Some(Instant::now()),
            last_audio_send: None,
        }
    }

    pub fn set_audio(&self, sink: Option<Box<dyn AudioStream>>) {
        *self.audio.borrow_mut() = sink;
    }

    pub fn active_party_count(&self) -> usize {
        self.parties
            .iter()
            .filter(|p| match p.status {
                CallPartyStatus::Active => true,
                _ => false,
            })
            .count()
    }

    pub fn active_or_ringing_party_count(&self) -> usize {
        self.parties
            .iter()
            .filter(|p| match p.status {
                CallPartyStatus::Ringing => true,
                CallPartyStatus::Active => true,
                _ => false,
            })
            .count()
    }

    pub fn update_last_ring(&mut self) {
        self.last_ring = Some(Instant::now());
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

    pub fn set_party_status(&mut self, public_key: &str, status: CallPartyStatus) {
        if let Some(party) = self
            .parties
            .iter_mut()
            .find(|p| p.contact.public_key == public_key)
        {
            party.status = status;
        }
    }

    pub fn add_party(&mut self, party: CallParty) -> bool {
        if self.get_status(&party.contact.public_key).is_some() {
            return false;
        }
        self.parties.push(party);
        true
    }

    pub fn get_call_label(&self, own_contact: Contact) -> String {
        let names = self
            .parties
            .iter()
            .filter(|p| p.contact.public_key != own_contact.public_key)
            .map(|p| p.contact.alias.clone())
            .collect::<Vec<_>>();

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
                CallPartyStatus::Ringing => {
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
        // --- Invite sending (ringing parties) ---
        let now = Instant::now();
        let should_send = self
            .last_invite
            .map_or(true, |t| now.duration_since(t).as_secs() >= 1);

        if should_send {
            self.last_invite = Some(now);
            for party in &self.parties {
                if matches!(party.status, CallPartyStatus::Ringing) {
                    client.net.send(HoshiMessage::new(
                        client.public_key(),
                        party.contact.public_key.clone(),
                        HoshiPayload::InviteToCall {
                            call_id: self.id.clone(),
                        },
                    ));
                }
            }
        }

        // --- Audio: start/stop sink and source based on party status ---
        if self.parties.len() > 1 {
            self.audio.borrow().as_ref().map(|s| s.play());
        } else {
            self.audio.borrow().as_ref().map(|s| s.pause());
        }

        // --- Capture and send audio every 20ms ---
        let should_send_audio = self
            .last_audio_send
            .map_or(true, |t| t.elapsed().as_millis() >= 20);
        if !should_send_audio {
            return;
        }

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
                .find(|c| matches!(c.status, CallPartyStatus::Ringing))
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

        let mut samples_48k = [0i16; 1024];
        if let Some(source) = self.audio.borrow().as_ref() {
            let samples_read = source.read(&mut samples_48k);
            // If we don't have enough data buffered we'll just return and send data the next time
            if samples_read == 0 {
                return;
            }
        }

        // Encode as μ-law
        let encoded: Vec<u8> = samples_48k.iter().map(|&s| linear_to_ulaw(s)).collect();

        let chunk = AudioChunk::ULaw {
            sample_rate: 48_000,
            samples: encoded,
        };
        self.last_audio_send = Some(Instant::now());

        for key in &active_keys {
            client.net.send(HoshiMessage::new(
                my_key.clone(),
                key.clone(),
                HoshiPayload::AudioChunk {
                    call_id: self.id.clone(),
                    chunk: chunk.clone(),
                },
            ));
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
