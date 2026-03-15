use anyhow::Result;

use crate::{Call, HoshiClient};

pub trait AudioStream: std::fmt::Debug {
    /// Gets called by the clientlib and the samples should be played back over the
    /// associated sink
    fn write(&self, channel: usize, samples: &[i16]) -> usize;

    /// Gets called by the clientlib if it needs audio data for a call, if paued should fill
    /// buf with 0
    fn read(&self, buf: &mut [i16]) -> usize;

    /// Gets called by the clientlib before we start calling write
    ///
    /// Can safely be called if already playing
    fn play(&self);

    /// Gets called by the clientlib after the last call to write, if a call ends
    ///
    /// Can safely be called when already paused
    fn pause(&self);

    /// Returns true if the underlying audio device has encountered an error.
    /// The clientlib will drop and recreate the stream when this returns true.
    fn has_error(&self) -> bool {
        false
    }
}

pub trait AudioInterface: std::fmt::Debug {
    /// The main entry point, the idea is that whenever we start/get a new call we create
    /// a new audio interface, this allows clients to handle multiple calls and simplify
    /// various bot usecases (Echo Service for example). Additionally we only open a connection
    /// when it's needed not before.
    fn create(&self, client: &HoshiClient, call: &Call) -> Result<Box<dyn AudioStream>>;
}

/// To keep things simple we just force the sample rate to be 48KHz for now, should be good enough for voice calls
pub const AUDIO_INTERFACE_SAMPLE_RATE: u32 = 48_000;

/// We also enforce a single mono audio channel, since we only focus on voice calls in the beginning this is sufficient
pub const AUDIO_INTERFACE_CHANNEL_COUNT: u8 = 1;
