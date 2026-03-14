use std::{
    cell::RefCell,
    collections::{HashMap, VecDeque},
    sync::{Arc, Mutex},
};

use anyhow::Result;
use hoshi_clientlib::{AudioInterface, AudioStream, Call, HoshiClient};

#[derive(Debug)]
struct Loopback {
    playing: RefCell<bool>,
    buffers: Arc<Mutex<HashMap<usize, VecDeque<i16>>>>,
}

impl Loopback {
    pub fn new() -> Self {
        let sink_buffers: Arc<Mutex<HashMap<usize, VecDeque<i16>>>> =
            Arc::new(Mutex::new(HashMap::new()));

        Self {
            playing: RefCell::new(false),
            buffers: sink_buffers,
        }
    }
}

impl AudioStream for Loopback {
    fn write(&self, channel: usize, samples: &[i16]) -> usize {
        if !*self.playing.borrow() {
            return 0;
        }
        let mut buffers = self.buffers.lock().unwrap();
        let buf = buffers.entry(channel).or_insert_with(VecDeque::new);
        buf.extend(samples.iter().copied());
        // If buffer exceeds cap, drop oldest samples to bound playback latency.
        if buf.len() > 8192 {
            println!("Loopback Sink buffer exceeded cap!");
            let excess = buf.len() - 8192;
            buf.drain(..excess);
        }
        if buf.len() < 1024 {
            println!("Loopback Sink Buffer dangerously small");
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

        let mut buffers = self.buffers.lock().unwrap();
        for out in buf.iter_mut() {
            let mixed: i32 = buffers
                .values_mut()
                .map(|deque| deque.pop_front().unwrap_or(0i16) as i32)
                .sum();
            *out = mixed.clamp(i16::MIN as i32, i16::MAX as i32) as i16;
        }

        buf.len()
    }

    fn play(&self) {
        let mut playing = self.playing.borrow_mut();
        if !*playing {
            *playing = true;
        }
    }

    fn pause(&self) {
        let mut playing = self.playing.borrow_mut();
        if *playing {
            *playing = false;
        }
    }
}

#[derive(Debug)]
pub struct LoopbackInterface {}

impl LoopbackInterface {
    pub fn new() -> Self {
        Self {}
    }
}

impl AudioInterface for LoopbackInterface {
    fn create(&self, _client: &HoshiClient, _call: &Call) -> Result<Box<dyn AudioStream>> {
        let stream = Loopback::new();
        Ok(Box::new(stream))
    }
}
