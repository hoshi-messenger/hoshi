pub trait AudioInterfaceSink {
    /// Gets called by the clientlib and the samples should be played back over the
    /// associated sink
    fn write(&self, samples: &[i16]) -> usize;

    /// Gets called by the clientlib before we start calling write
    ///
    /// Can safely be called if already playing
    fn play(&self);

    /// Gets called by the clientlib after the last call to write, if a call ends
    ///
    /// Can safely be called when already paused
    fn pause(&self);
}

pub trait AudioInterfaceSource {
    /// Gets called by the clientlib if it needs audio data for a call, if paued should fill
    /// buf with 0
    fn read(&self, buf: &mut [i16]) -> usize;

    /// Gets called by the clientlib before we call read, mainly because a call is about to start
    ///
    /// Can safely be called if already playing
    fn play(&self);

    /// Gets called by the clientlib after a call concluded and we'll stop calling read for a while
    ///
    /// Can safely be called if already paused
    fn pause(&self);
}

/// To keep things simple we just force the sample rate to be 48KHz for now, should be good enough for voice calls
pub const AUDIO_INTERFACE_SAMPLE_RATE: u32 = 48_000;

/// We also enforce a single mono audio channel, since we only focus on voice calls in the beginning this is sufficient
pub const AUDIO_INTERFACE_CHANNEL_COUNT: u8 = 1;
