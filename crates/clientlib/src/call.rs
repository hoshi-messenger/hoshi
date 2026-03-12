use std::time::Instant;

use serde::{Deserialize, Serialize};

use crate::{
    Contact,
    audio_chunk::{AudioChunk, linear_to_ulaw},
    hoshi_client::HoshiClient,
    hoshi_net_client::{HoshiMessage, HoshiPayload},
};

#[derive(Clone, Debug)]
pub struct Call {
    id: String,
    parties: Vec<CallParty>,

    last_audio_send: Option<Instant>,
    last_invite: Option<Instant>,
    pub call_started: Option<Instant>,
    pub call_ended: Option<Instant>,
    last_ring: Option<Instant>,
}

impl Call {
    pub fn new(parties: Vec<Contact>) -> Self {
        let parties = parties.into_iter().map(|p| p.into()).collect();
        Self {
            id: uuid::Uuid::now_v7().to_string(),
            parties,
            last_invite: None,
            call_started: Some(Instant::now()),
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
            last_invite: None,
            call_started: Some(Instant::now()),
            call_ended: None,
            last_ring: Some(Instant::now()),
            last_audio_send: None,
        }
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

    pub fn is_ring_timed_out(&self) -> bool {
        self.last_ring.map_or(false, |t| t.elapsed().as_secs() >= 3)
    }

    pub fn should_auto_close(&self) -> bool {
        self.call_ended
            .map_or(false, |t| t.elapsed().as_secs() >= 5)
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
                        let s = self
                            .call_started
                            .map(|s| s.elapsed().as_secs())
                            .unwrap_or_default();
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

    pub fn get_party_status_pairs(&self) -> Vec<(String, CallPartyStatus)> {
        self.parties
            .iter()
            .map(|p| (p.contact.alias.clone(), p.status))
            .collect()
    }

    pub fn get_parties(&self) -> Vec<CallParty> {
        self.parties.clone()
    }

    pub fn get_voice_activity(&self, public_key: &str) -> f32 {
        self.parties
            .iter()
            .find(|p| p.contact.public_key == public_key)
            .map(|p| p.voice_activity)
            .unwrap_or(0.0)
    }

    pub fn stop(&self) {
        // ToDo: inform other parties
    }

    /// Receives an incoming audio chunk, decodes it, upsamples from 24kHz to 48kHz,
    /// and writes it to the audio sink.
    pub fn receive_audio(&mut self, chunk: AudioChunk, from_key: &str, client: &HoshiClient) {
        let decoded = chunk.decode_i16(); // i16 samples at 24kHz

        // RMS voice activity
        let activity = if decoded.is_empty() {
            0.0
        } else {
            let sum_sq: f32 = decoded
                .iter()
                .map(|&s| {
                    let f = s as f32 / 32768.0;
                    f * f
                })
                .sum();
            (sum_sq / decoded.len() as f32).sqrt()
        };

        if let Some(party) = self
            .parties
            .iter_mut()
            .find(|p| p.contact.public_key == from_key)
        {
            party.voice_activity = activity;
        }

        // Upsample 24kHz → 48kHz by repeating each sample
        let upsampled: Vec<i16> = decoded.iter().flat_map(|&s| [s, s]).collect();

        if let Some(sink) = client.audio_sink.borrow().as_ref() {
            sink.write(&upsampled);
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
            client.audio_sink.borrow().as_ref().map(|s| s.play());
            client.audio_source.borrow().as_ref().map(|s| s.play());
        } else {
            client.audio_sink.borrow().as_ref().map(|s| s.pause());
            client.audio_source.borrow().as_ref().map(|s| s.pause());
        }

        // --- Capture and send audio every 20ms ---
        let should_send_audio = self
            .last_audio_send
            .map_or(true, |t| t.elapsed().as_millis() >= 20);
        if !should_send_audio {
            return;
        }
        self.last_audio_send = Some(Instant::now());

        let my_key = client.public_key();
        let call_id = self.id.clone();
        let active_keys: Vec<String> = self
            .parties
            .iter()
            .filter(|p| matches!(p.status, CallPartyStatus::Active))
            .map(|p| p.contact.public_key.clone())
            .collect();

        // Read 20ms of mono i16 audio at 48kHz (960 samples)
        let mut buf_48 = [0i16; 960];
        if let Some(source) = client.audio_source.borrow().as_ref() {
            source.read(&mut buf_48);
        }

        // Downsample 48kHz → 24kHz by averaging pairs of samples
        let samples_24: Vec<i16> = buf_48
            .chunks(2)
            .map(|pair| ((pair[0] as i32 + pair[1] as i32) / 2) as i16)
            .collect();

        // Encode as μ-law
        let encoded: Vec<u8> = samples_24.iter().map(|&s| linear_to_ulaw(s)).collect();

        let chunk = AudioChunk::ULaw {
            id: call_id.clone(),
            chunk_offset: 0,
            sample_rate: 24_000,
            samples: encoded,
        };

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
    /// RMS of the last received AudioChunk, 0.0–1.0.
    pub voice_activity: f32,
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
            voice_activity: 0.0,
        }
    }
}
