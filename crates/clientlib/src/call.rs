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
    last_invite: Option<Instant>,
    pub call_started: Option<Instant>,
    pub call_ended: Option<Instant>,
    last_ring: Option<Instant>,
    chunk_offset: i32,
    local_voice_activity: f32,
    audio_started: bool,
    last_audio_send: Option<Instant>,
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
            chunk_offset: 0,
            local_voice_activity: 0.0,
            audio_started: false,
            last_audio_send: None,
        }
    }

    pub fn from_invite(id: String, caller: Contact) -> Self {
        Self {
            id,
            parties: vec![caller.into()],
            last_invite: None,
            call_started: Some(Instant::now()),
            call_ended: None,
            last_ring: Some(Instant::now()),
            chunk_offset: 0,
            local_voice_activity: 0.0,
            audio_started: false,
            last_audio_send: None,
        }
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

    pub fn add_party(&mut self, contact: Contact) -> bool {
        if self.get_status(&contact.public_key).is_some() {
            return false;
        }
        self.parties.push(contact.into());
        true
    }

    pub fn get_party_names(&self) -> Vec<String> {
        self.parties
            .iter()
            .map(|p| p.contact.alias.clone())
            .collect()
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

    pub fn get_local_voice_activity(&self) -> f32 {
        self.local_voice_activity
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

    fn all_parties_active(&self) -> bool {
        !self.parties.is_empty()
            && self
                .parties
                .iter()
                .all(|p| matches!(p.status, CallPartyStatus::Active))
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
                            from_key: client.public_key(),
                            id: self.id.clone(),
                        },
                    ));
                }
            }
        }

        // --- Audio: start/stop sink and source based on party status ---
        let all_active = self.all_parties_active();
        if all_active && !self.audio_started {
            self.audio_started = true;
            if let Some(sink) = client.audio_sink.borrow().as_ref() {
                sink.play();
            }
            if let Some(source) = client.audio_source.borrow().as_ref() {
                source.play();
            }
        } else if !all_active && self.audio_started {
            self.audio_started = false;
            if let Some(sink) = client.audio_sink.borrow().as_ref() {
                sink.pause();
            }
            if let Some(source) = client.audio_source.borrow().as_ref() {
                source.pause();
            }
        }

        if !self.audio_started {
            return;
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

        // Voice activity on outgoing audio
        if !samples_24.is_empty() {
            let sum_sq: f32 = samples_24
                .iter()
                .map(|&s| {
                    let f = s as f32 / 32768.0;
                    f * f
                })
                .sum();
            self.local_voice_activity = (sum_sq / samples_24.len() as f32).sqrt();
        }

        // Encode as μ-law
        let encoded: Vec<u8> = samples_24.iter().map(|&s| linear_to_ulaw(s)).collect();

        let offset = self.chunk_offset;
        self.chunk_offset += 1;

        let chunk = AudioChunk::ULaw {
            id: call_id.clone(),
            chunk_offset: offset,
            sample_rate: 24_000,
            samples: encoded,
        };

        for key in &active_keys {
            client.net.send(HoshiMessage::new(
                my_key.clone(),
                key.clone(),
                HoshiPayload::AudioChunk(chunk.clone()),
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
