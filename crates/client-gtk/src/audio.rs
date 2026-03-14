use std::cell::RefCell;
use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};

use anyhow::{Result, anyhow};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, Sample, SampleFormat, Stream};

use hoshi_clientlib::{
    AUDIO_INTERFACE_SAMPLE_RATE, AudioInterface, AudioStream, Call, HoshiClient,
};

use crate::AppState;

/// Sink cap: ~100ms at 48kHz. Bounds playback latency; older samples are dropped when exceeded.
const SINK_BUFFER_CAP: usize = 8192*2;
/// Source cap: ~100ms at 48kHz. Oldest samples are dropped when exceeded to keep audio live.
const SOURCE_BUFFER_CAP: usize = 4096;

struct ClientStream {
    source_stream: Stream,
    sink_stream: Stream,
    playing: RefCell<bool>,
    source_buffer: Arc<Mutex<VecDeque<i16>>>,
    sink_buffers: Arc<Mutex<HashMap<usize, VecDeque<i16>>>>,
}

impl std::fmt::Debug for ClientStream {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "ClientSink")
    }
}

impl AudioStream for ClientStream {
    fn write(&self, channel: usize, samples: &[i16]) -> usize {
        if !*self.playing.borrow() {
            return 0;
        }
        let mut buffers = self.sink_buffers.lock().unwrap();
        let buf = buffers.entry(channel).or_insert_with(|| {
            let mut deque = VecDeque::new();
            // Super simple static jitter buffer, should make this more sophisticated in the long run
            deque.extend([0i16; 2048].into_iter());
            deque
        });
        buf.extend(samples.iter().copied());
        // If buffer exceeds cap, drop oldest samples to bound playback latency.
        if buf.len() > SINK_BUFFER_CAP {
            println!("Sink buffer exceeded cap!");
            let excess = buf.len() - SINK_BUFFER_CAP;
            buf.drain(..excess);
        }
        samples.len()
    }

    fn read(&self, buf: &mut [i16]) -> usize {
        if !*self.playing.borrow() {
            for s in buf.iter_mut() {
                *s = 0;
            }
            return buf.len();
        }
        let mut buffer = self.source_buffer.lock().unwrap();
        if buffer.len() < buf.len() {
            return 0;
        }
        for s in buf.iter_mut() {
            *s = buffer.pop_front().unwrap_or(0);
        }
        buf.len()
    }

    fn play(&self) {
        let mut playing = self.playing.borrow_mut();
        if !*playing {
            let _ = self.sink_stream.play();
            let _ = self.source_stream.play();
            *playing = true;
        }
    }

    fn pause(&self) {
        let mut playing = self.playing.borrow_mut();
        if *playing {
            let _ = self.sink_stream.pause();
            let _ = self.source_stream.pause();
            *playing = false;
        }
    }
}

fn fill_output<T: Sample + FromSample<i16>>(
    data: &mut [T],
    buffers: &Mutex<HashMap<usize, VecDeque<i16>>>,
) {
    let mut buffers = buffers.lock().unwrap();
    /*
    let mut buffers = match buffers.try_lock() {
        Ok(b) => b,
        Err(_) => {
            eprintln!("MUTEX CONTENDED in callback!");
            // fill with silence and return
            data.iter_mut().for_each(|s| *s = T::from_sample(0i16));
            return;
        }
    };

    let sizes: Vec<_> = buffers.iter().map(|(k, v)| (k, v.len())).collect();
    eprintln!("callback: requesting {} samples, deque sizes: {:?}", data.len(), sizes);
     */

    for out in data.iter_mut() {
        let mixed: i32 = buffers
            .values_mut()
            .map(|deque| deque.pop_front().unwrap_or(0i16) as i32)
            .sum();
        *out = T::from_sample(mixed.clamp(i16::MIN as i32, i16::MAX as i32) as i16);
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

impl ClientStream {
    pub fn new() -> Result<Self> {
        let host = cpal::default_host();

        let sink_buffers: Arc<Mutex<HashMap<usize, VecDeque<i16>>>> =
            Arc::new(Mutex::new(HashMap::new()));

        let sink_stream = {
            let device = host
                .default_output_device()
                .ok_or(anyhow!("No output device"))?;

            let supported = device
                .supported_output_configs()?
                .find(|c| {
                    c.channels() == 1
                        && c.min_sample_rate().0 <= AUDIO_INTERFACE_SAMPLE_RATE
                        && c.max_sample_rate().0 >= AUDIO_INTERFACE_SAMPLE_RATE
                        && (matches!(c.sample_format(), SampleFormat::I16)
                            || matches!(c.sample_format(), SampleFormat::U16)
                            || matches!(c.sample_format(), SampleFormat::F32))
                })
                .ok_or(anyhow!("Output device does not support mono 48kHz"))?
                .with_sample_rate(cpal::SampleRate(AUDIO_INTERFACE_SAMPLE_RATE));

            println!("{:?}", supported);

            let sample_format = supported.sample_format();
            let config = cpal::StreamConfig {
                channels: 1,
                sample_rate: cpal::SampleRate(AUDIO_INTERFACE_SAMPLE_RATE),
                buffer_size: cpal::BufferSize::Fixed(4096),
            };

            let err_fn = |err| eprintln!("Output stream error: {err}");

            let b = Arc::clone(&sink_buffers);
            match sample_format {
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
                fmt => return Err(anyhow!("Unsupported output sample format: {fmt}")),
            }?
        };

        let source_buffer: Arc<Mutex<VecDeque<i16>>> = Arc::new(Mutex::new(VecDeque::new()));
        let source_stream = {
            let device = host
                .default_input_device()
                .ok_or(anyhow!("No input device"))?;

            let supported = device
                .supported_input_configs()?
                .find(|c| {
                    c.channels() == 1
                        && c.min_sample_rate().0 <= AUDIO_INTERFACE_SAMPLE_RATE
                        && c.max_sample_rate().0 >= AUDIO_INTERFACE_SAMPLE_RATE
                        && (matches!(c.sample_format(), SampleFormat::I16)
                            || matches!(c.sample_format(), SampleFormat::U16)
                            || matches!(c.sample_format(), SampleFormat::F32))
                })
                .ok_or(anyhow!("Input device does not support mono 48kHz"))?
                .with_sample_rate(cpal::SampleRate(AUDIO_INTERFACE_SAMPLE_RATE));

            let sample_format = supported.sample_format();
            let config = cpal::StreamConfig {
                channels: 1,
                sample_rate: cpal::SampleRate(AUDIO_INTERFACE_SAMPLE_RATE),
                buffer_size: cpal::BufferSize::Fixed(4096),
            };

            let err_fn = |err| eprintln!("Input stream error: {err}");

            let b = Arc::clone(&source_buffer);
            match sample_format {
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
                fmt => return Err(anyhow!("Unsupported input sample format: {fmt}")),
            }?
        };

        Ok(Self {
            source_stream,
            sink_stream,
            playing: RefCell::new(false),
            source_buffer,
            sink_buffers,
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
    fn create(&self, _client: &HoshiClient, _call: &Call) -> Result<Box<dyn AudioStream>> {
        let stream = ClientStream::new()?;

        Ok(Box::new(stream))
    }
}

pub fn init_audio_interfaces(state: AppState) -> Result<()> {
    let interface = ClientInterface::new();
    state.client.set_audio_interface(Some(Box::new(interface)));
    Ok(())
}
