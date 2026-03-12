use std::cell::RefCell;
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use anyhow::{Result, anyhow};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, Sample, SampleFormat, Stream};

use hoshi_clientlib::{
    AUDIO_INTERFACE_SAMPLE_RATE, AudioInterface, AudioInterfaceSink, AudioInterfaceSource, Call,
    HoshiClient,
};

use crate::AppState;

/// Sink cap: ~200ms at 48kHz. Bounds playback latency; older samples are dropped when exceeded.
const SINK_BUFFER_CAP: usize = 9_600;
/// Source cap: ~100ms at 48kHz. Oldest samples are dropped when exceeded to keep audio live.
const SOURCE_BUFFER_CAP: usize = 4_800;

struct ClientSink {
    stream: Stream,
    playing: RefCell<bool>,
    buffer: Arc<Mutex<VecDeque<i16>>>,
}

impl std::fmt::Debug for ClientSink {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "ClientSink")
    }
}

impl AudioInterfaceSink for ClientSink {
    fn write(&self, samples: &[i16]) -> usize {
        if !*self.playing.borrow() {
            return 0;
        }
        let mut buf = self.buffer.lock().unwrap();
        buf.extend(samples.iter().copied());
        // If buffer exceeds cap, drop oldest samples to bound playback latency.
        if buf.len() > SINK_BUFFER_CAP {
            let excess = buf.len() - SINK_BUFFER_CAP;
            buf.drain(..excess);
        }
        samples.len()
    }

    fn play(&self) {
        let mut playing = self.playing.borrow_mut();
        if !*playing {
            let _ = self.stream.play();
            *playing = true;
        }
    }

    fn pause(&self) {
        let mut playing = self.playing.borrow_mut();
        if *playing {
            let _ = self.stream.pause();
            *playing = false;
        }
    }
}

struct ClientSource {
    stream: Stream,
    playing: RefCell<bool>,
    buffer: Arc<Mutex<VecDeque<i16>>>,
}

impl std::fmt::Debug for ClientSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "ClientSource")
    }
}

impl AudioInterfaceSource for ClientSource {
    fn read(&self, buf: &mut [i16]) -> usize {
        if !*self.playing.borrow() {
            for s in buf.iter_mut() {
                *s = 0;
            }
            return buf.len();
        }
        let mut buffer = self.buffer.lock().unwrap();
        for s in buf.iter_mut() {
            *s = buffer.pop_front().unwrap_or(0);
        }
        buf.len()
    }

    fn play(&self) {
        let mut playing = self.playing.borrow_mut();
        if !*playing {
            let _ = self.stream.play();
            *playing = true;
        }
    }

    fn pause(&self) {
        let mut playing = self.playing.borrow_mut();
        if *playing {
            let _ = self.stream.pause();
            *playing = false;
        }
    }
}

fn fill_output<T: Sample + FromSample<i16>>(data: &mut [T], buffer: &Mutex<VecDeque<i16>>) {
    let mut buf = buffer.lock().unwrap();
    for out in data.iter_mut() {
        *out = T::from_sample(buf.pop_front().unwrap_or(0i16));
    }
}

fn fill_input<T: Sample>(data: &[T], buffer: &Mutex<VecDeque<i16>>)
where
    i16: FromSample<T>,
{
    let mut buf = buffer.lock().unwrap();
    for &s in data {
        buf.push_back(i16::from_sample(s));
        if buf.len() > SOURCE_BUFFER_CAP {
            buf.pop_front();
        }
    }
}

impl ClientSink {
    pub fn new() -> Result<Self> {
        let host = cpal::default_host();
        let device = host
            .default_output_device()
            .ok_or(anyhow!("No output device"))?;

        let supported = device
            .supported_output_configs()?
            .find(|c| {
                c.channels() == 1
                    && c.min_sample_rate().0 <= AUDIO_INTERFACE_SAMPLE_RATE
                    && c.max_sample_rate().0 >= AUDIO_INTERFACE_SAMPLE_RATE
            })
            .ok_or(anyhow!("Output device does not support mono 48kHz"))?
            .with_sample_rate(cpal::SampleRate(AUDIO_INTERFACE_SAMPLE_RATE));

        let sample_format = supported.sample_format();
        let config: cpal::StreamConfig = supported.into();

        let buffer: Arc<Mutex<VecDeque<i16>>> = Arc::new(Mutex::new(VecDeque::new()));

        let err_fn = |err| eprintln!("Output stream error: {err}");

        let b = Arc::clone(&buffer);
        let stream = match sample_format {
            SampleFormat::F32 => device.build_output_stream(
                &config,
                move |data: &mut [f32], _| fill_output(data, &b),
                err_fn,
                None,
            ),
            SampleFormat::I16 => device.build_output_stream(
                &config,
                move |data: &mut [i16], _| fill_output(data, &b),
                err_fn,
                None,
            ),
            SampleFormat::U16 => device.build_output_stream(
                &config,
                move |data: &mut [u16], _| fill_output(data, &b),
                err_fn,
                None,
            ),
            SampleFormat::U8 => device.build_output_stream(
                &config,
                move |data: &mut [u8], _| fill_output(data, &b),
                err_fn,
                None,
            ),
            fmt => return Err(anyhow!("Unsupported output sample format: {fmt}")),
        }?;

        Ok(Self {
            stream,
            playing: RefCell::new(false),
            buffer,
        })
    }
}

impl ClientSource {
    pub fn new() -> Result<Self> {
        let host = cpal::default_host();
        let device = host
            .default_input_device()
            .ok_or(anyhow!("No input device"))?;

        let supported = device
            .supported_input_configs()?
            .find(|c| {
                c.channels() == 1
                    && c.min_sample_rate().0 <= AUDIO_INTERFACE_SAMPLE_RATE
                    && c.max_sample_rate().0 >= AUDIO_INTERFACE_SAMPLE_RATE
            })
            .ok_or(anyhow!("Input device does not support mono 48kHz"))?
            .with_sample_rate(cpal::SampleRate(AUDIO_INTERFACE_SAMPLE_RATE));

        let sample_format = supported.sample_format();
        let config: cpal::StreamConfig = supported.into();

        let buffer: Arc<Mutex<VecDeque<i16>>> = Arc::new(Mutex::new(VecDeque::new()));

        let err_fn = |err| eprintln!("Input stream error: {err}");

        let b = Arc::clone(&buffer);
        let stream = match sample_format {
            SampleFormat::F32 => device.build_input_stream(
                &config,
                move |data: &[f32], _| fill_input(data, &b),
                err_fn,
                None,
            ),
            SampleFormat::I16 => device.build_input_stream(
                &config,
                move |data: &[i16], _| fill_input(data, &b),
                err_fn,
                None,
            ),
            SampleFormat::U16 => device.build_input_stream(
                &config,
                move |data: &[u16], _| fill_input(data, &b),
                err_fn,
                None,
            ),
            SampleFormat::U8 => device.build_input_stream(
                &config,
                move |data: &[u8], _| fill_input(data, &b),
                err_fn,
                None,
            ),
            fmt => return Err(anyhow!("Unsupported input sample format: {fmt}")),
        }?;

        Ok(Self {
            stream,
            playing: RefCell::new(false),
            buffer,
        })
    }
}

#[derive(Debug)]
struct ClientInterface {}

impl ClientInterface {
    pub fn new() -> Self {
        Self {}
    }
}

impl AudioInterface for ClientInterface {
    fn create(
        &self,
        _client: &HoshiClient,
        _call: &Call,
    ) -> Result<(Box<dyn AudioInterfaceSink>, Box<dyn AudioInterfaceSource>)> {
        let sink = ClientSink::new()?;
        let source = ClientSource::new()?;

        Ok((Box::new(sink), Box::new(source)))
    }
}

pub fn init_audio_interfaces(state: AppState) -> Result<()> {
    let interface = ClientInterface::new();
    state.client.set_audio_interface(Some(Box::new(interface)));
    Ok(())
}
