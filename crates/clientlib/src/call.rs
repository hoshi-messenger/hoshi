use std::cell::RefCell;

use serde::{Deserialize, Serialize};

use crate::{
    Contact,
    audio_chunk::{AudioChunk, linear_to_ulaw},
    hoshi_client::HoshiClient,
    hoshi_net_client::{HoshiMessage, HoshiPayload},
};

struct AudioCapture {
    /// Keeps the cpal stream alive; dropped when audio stops.
    _stream: cpal::Stream,
    /// Receives μ-law encoded packets of exactly 2048 samples from the resampling thread.
    rx: std::sync::mpsc::Receiver<Vec<u8>>,
}

struct AudioPlayback {
    /// Keeps the rodio output stream alive.
    _stream: rodio::OutputStream,
    sink: rodio::Sink,
}

#[derive(Debug)]
pub struct Call {
    id: String,
    parties: RefCell<Vec<CallParty>>,
    last_invite: RefCell<Option<std::time::Instant>>,
    pub call_started: RefCell<Option<std::time::Instant>>,
    pub call_ended: RefCell<Option<std::time::Instant>>,
    last_ring: RefCell<Option<std::time::Instant>>,
    audio: RefCell<Option<(AudioCapture, AudioPlayback)>>,
    chunk_offset: RefCell<i32>,
    local_voice_activity: RefCell<f32>,
}

impl Clone for Call {
    fn clone(&self) -> Self {
        Self {
            id: self.id.clone(),
            parties: self.parties.clone(),
            last_invite: self.last_invite.clone(),
            call_started: self.call_started.clone(),
            call_ended: self.call_ended.clone(),
            last_ring: self.last_ring.clone(),
            // Audio state is not cloned — the clone starts silent.
            audio: RefCell::new(None),
            chunk_offset: self.chunk_offset.clone(),
            local_voice_activity: self.local_voice_activity.clone(),
        }
    }
}

impl std::fmt::Debug for AudioCapture {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("AudioCapture")
    }
}

impl std::fmt::Debug for AudioPlayback {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("AudioPlayback")
    }
}

impl Call {
    pub fn new(parties: Vec<Contact>) -> Self {
        let parties = parties
            .into_iter()
            .map(|p| p.into())
            .collect::<Vec<CallParty>>();
        let parties = RefCell::new(parties);
        let id = uuid::Uuid::now_v7().to_string();
        Self {
            id,
            parties,
            last_invite: RefCell::new(None),
            call_started: RefCell::new(Some(std::time::Instant::now())),
            call_ended: RefCell::new(None),
            last_ring: RefCell::new(None),
            audio: RefCell::new(None),
            chunk_offset: RefCell::new(0),
            local_voice_activity: RefCell::new(0.0),
        }
    }

    pub fn from_invite(id: String, caller: Contact) -> Self {
        Self {
            id,
            parties: RefCell::new(vec![caller.into()]),
            last_invite: RefCell::new(None),
            call_started: RefCell::new(Some(std::time::Instant::now())),
            call_ended: RefCell::new(None),
            last_ring: RefCell::new(Some(std::time::Instant::now())),
            audio: RefCell::new(None),
            chunk_offset: RefCell::new(0),
            local_voice_activity: RefCell::new(0.0),
        }
    }

    pub fn update_last_ring(&self) {
        *self.last_ring.borrow_mut() = Some(std::time::Instant::now());
    }

    pub fn is_ring_timed_out(&self) -> bool {
        self.last_ring
            .borrow()
            .map_or(false, |t| t.elapsed().as_secs() >= 3)
    }

    pub fn should_auto_close(&self) -> bool {
        self.call_ended
            .borrow()
            .map_or(false, |t| t.elapsed().as_secs() >= 5)
    }

    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn get_status(&self, public_key: &str) -> Option<CallPartyStatus> {
        for party in self.parties.borrow().iter() {
            if &party.contact.public_key == public_key {
                return Some(party.status);
            }
        }
        None
    }

    pub fn set_party_status(&self, public_key: &str, status: CallPartyStatus) {
        for party in self.parties.borrow_mut().iter_mut() {
            if party.contact.public_key == public_key {
                party.status = status;
                return;
            }
        }
    }

    pub fn add_party(&self, contact: Contact) -> bool {
        if self.get_status(&contact.public_key).is_some() {
            return false;
        }
        self.parties.borrow_mut().push(contact.into());
        true
    }

    pub fn get_party_names(&self) -> Vec<String> {
        self.parties
            .borrow()
            .iter()
            .map(|p| p.contact.alias.clone())
            .collect()
    }

    pub fn get_party_public_keys(&self) -> Vec<String> {
        self.parties
            .borrow()
            .iter()
            .map(|p| p.contact.public_key.clone())
            .collect()
    }

    pub fn get_party_status_pairs(&self) -> Vec<(String, CallPartyStatus)> {
        self.parties
            .borrow()
            .iter()
            .map(|p| (p.contact.alias.clone(), p.status))
            .collect()
    }

    pub fn get_parties(&self) -> Vec<CallParty> {
        self.parties.borrow().clone()
    }

    pub fn get_local_voice_activity(&self) -> f32 {
        *self.local_voice_activity.borrow()
    }

    pub fn get_voice_activity(&self, public_key: &str) -> f32 {
        self.parties
            .borrow()
            .iter()
            .find(|p| p.contact.public_key == public_key)
            .map(|p| p.voice_activity)
            .unwrap_or(0.0)
    }

    pub fn stop(&self) {
        // ToDo: inform other parties
    }

    pub fn receive_audio(&self, chunk: AudioChunk, from_key: &str) {
        let decoded = chunk.decode_f32();

        // RMS of this chunk as a rough voice activity level
        let activity = if decoded.is_empty() {
            0.0
        } else {
            let sum_sq: f32 = decoded.iter().map(|&s| s * s).sum();
            (sum_sq / decoded.len() as f32).sqrt()
        };

        for party in self.parties.borrow_mut().iter_mut() {
            if party.contact.public_key == from_key {
                party.voice_activity = activity;
                break;
            }
        }

        if let Some((_, playback)) = self.audio.borrow().as_ref() {
            playback.sink.append(rodio::buffer::SamplesBuffer::new(
                1,
                chunk.sample_rate() as u32,
                decoded,
            ));
        }
    }

    fn all_parties_active(&self) -> bool {
        let parties = self.parties.borrow();
        !parties.is_empty()
            && parties
                .iter()
                .all(|p| matches!(p.status, CallPartyStatus::Active))
    }

    fn start_audio() -> Option<(AudioCapture, AudioPlayback)> {
        use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};

        let host = cpal::default_host();
        let in_device = match host.default_input_device() {
            Some(d) => d,
            None => {
                eprintln!("No default input device available");
                return None;
            }
        };
        let in_config = match in_device.default_input_config() {
            Ok(c) => c,
            Err(e) => {
                eprintln!("Failed to get default input config: {e}");
                return None;
            }
        };
        let in_rate = in_config.sample_rate().0;
        let channels = in_config.channels() as usize;
        let sample_format = in_config.sample_format();
        let stream_config: cpal::StreamConfig = in_config.into();

        // raw mono f32 samples from cpal callback → resampling thread
        let (raw_tx, raw_rx) = std::sync::mpsc::sync_channel::<Vec<f32>>(32);
        // encoded 2048-sample μ-law packets → Call::step
        let (encoded_tx, encoded_rx) = std::sync::mpsc::sync_channel::<Vec<u8>>(32);

        // Resampling thread: rubato FftFixedOut resamples hw_rate → 32 kHz
        std::thread::spawn(move || {
            use rubato::{FftFixedOut, Resampler};
            let mut resampler = match FftFixedOut::<f32>::new(in_rate as usize, 32_000, 2048, 2, 1)
            {
                Ok(r) => r,
                Err(e) => {
                    eprintln!("rubato init error: {e}");
                    return;
                }
            };

            let mut accum: Vec<f32> = Vec::new();

            while let Ok(samples) = raw_rx.recv() {
                accum.extend_from_slice(&samples);
                loop {
                    let needed = resampler.input_frames_next();
                    if accum.len() < needed {
                        break;
                    }
                    let chunk: Vec<f32> = accum.drain(..needed).collect();
                    match resampler.process(&[&chunk], None) {
                        Ok(output) => {
                            let encoded: Vec<u8> = output[0]
                                .iter()
                                .map(|&s| linear_to_ulaw((s.clamp(-1.0, 1.0) * 32767.0) as i16))
                                .collect();
                            let _ = encoded_tx.try_send(encoded);
                        }
                        Err(e) => eprintln!("rubato process error: {e}"),
                    }
                }
            }
        });

        let stream = match sample_format {
            cpal::SampleFormat::F32 => in_device.build_input_stream(
                &stream_config,
                move |data: &[f32], _: &cpal::InputCallbackInfo| {
                    let mono: Vec<f32> = data
                        .chunks(channels)
                        .map(|c| c.iter().sum::<f32>() / channels as f32)
                        .collect();
                    let _ = raw_tx.try_send(mono);
                },
                |e| eprintln!("cpal input stream error: {e}"),
                None,
            ),
            cpal::SampleFormat::I16 => in_device.build_input_stream(
                &stream_config,
                move |data: &[i16], _: &cpal::InputCallbackInfo| {
                    let mono: Vec<f32> = data
                        .chunks(channels)
                        .map(|c| {
                            c.iter().map(|&s| s as f32 / 32768.0).sum::<f32>() / channels as f32
                        })
                        .collect();
                    let _ = raw_tx.try_send(mono);
                },
                |e| eprintln!("cpal input stream error: {e}"),
                None,
            ),
            fmt => {
                eprintln!("Unsupported input sample format: {fmt:?}");
                return None;
            }
        };

        let stream = match stream {
            Ok(s) => s,
            Err(e) => {
                eprintln!("Failed to build input stream: {e}");
                return None;
            }
        };

        if let Err(e) = stream.play() {
            eprintln!("Failed to start input stream: {e}");
            return None;
        }

        let (out_stream, handle) = match rodio::OutputStream::try_default() {
            Ok(v) => v,
            Err(e) => {
                eprintln!("Failed to open audio output: {e}");
                return None;
            }
        };
        let sink = match rodio::Sink::try_new(&handle) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("Failed to create audio sink: {e}");
                return None;
            }
        };

        Some((
            AudioCapture {
                _stream: stream,
                rx: encoded_rx,
            },
            AudioPlayback {
                _stream: out_stream,
                sink,
            },
        ))
    }

    pub fn step(&self, client: &HoshiClient) {
        // --- Invite sending (ringing parties) ---
        let now = std::time::Instant::now();
        let should_send = self
            .last_invite
            .borrow()
            .map_or(true, |t| now.duration_since(t).as_secs() >= 1);

        if should_send {
            *self.last_invite.borrow_mut() = Some(now);
            for party in self.parties.borrow().iter() {
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

        // --- Audio: start when all parties are active, stop otherwise ---
        if self.all_parties_active() {
            if self.audio.borrow().is_none() {
                *self.audio.borrow_mut() = Self::start_audio();
            }
        } else {
            self.audio.borrow_mut().take();
        }

        // --- Drain captured packets and send to all active parties ---
        let my_key = client.public_key();
        let call_id = self.id.clone();
        let active_keys: Vec<String> = self
            .parties
            .borrow()
            .iter()
            .filter(|p| matches!(p.status, CallPartyStatus::Active))
            .map(|p| p.contact.public_key.clone())
            .collect();

        if let Some((capture, _)) = self.audio.borrow().as_ref() {
            while let Ok(encoded) = capture.rx.try_recv() {
                let offset = {
                    let mut o = self.chunk_offset.borrow_mut();
                    let v = *o;
                    *o += 1;
                    v
                };
                let chunk = AudioChunk::ULaw {
                    id: call_id.clone(),
                    chunk_offset: offset,
                    sample_rate: 32_000,
                    samples: encoded,
                };

                // Measure local voice activity from the outgoing audio.
                let decoded = chunk.decode_f32();
                if !decoded.is_empty() {
                    let sum_sq: f32 = decoded.iter().map(|&s| s * s).sum();
                    *self.local_voice_activity.borrow_mut() =
                        (sum_sq / decoded.len() as f32).sqrt();
                }

                for key in &active_keys {
                    client.net.send(HoshiMessage::new(
                        my_key.clone(),
                        key.clone(),
                        HoshiPayload::AudioChunk(chunk.clone()),
                    ));
                }
            }
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
