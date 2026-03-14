use std::{
    cell::RefCell,
    io::BufReader,
    num::{NonZeroU16, NonZeroU32},
    path::PathBuf,
};

use anyhow::{Result, anyhow};
use hoshi_clientlib::{AudioInterface, AudioStream, Call, HoshiClient};
use rodio::{Decoder, conversions::SampleTypeConverter, source::UniformSourceIterator};

struct Jukebox {
    music_library: PathBuf,
    playing: RefCell<bool>,
    source: RefCell<Option<Box<dyn Iterator<Item = i16> + Send>>>,
}

impl std::fmt::Debug for Jukebox {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Jukebox")
    }
}

impl Jukebox {
    pub fn new(music_library: PathBuf) -> Self {
        Self {
            playing: RefCell::new(false),
            music_library,
            source: RefCell::new(None),
        }
    }

    fn load_next(&self) -> Result<()> {
        let entries: Vec<_> = match std::fs::read_dir(&self.music_library) {
            Ok(d) => d
                .filter_map(|e| e.ok())
                .filter(|e| {
                    matches!(
                        e.path().extension().and_then(|x| x.to_str()),
                        Some("mp3" | "m4a")
                    )
                })
                .collect(),
            Err(e) => {
                return Err(e.into());
            }
        };
        if entries.is_empty() {
            println!("No tracks found");
            return Err(anyhow!("No tracks found"));
        }

        let path = entries[(rand::random::<u32>() % entries.len() as u32) as usize].path();
        println!("Queuing: {:?}", &path);

        match std::fs::File::open(&path)
            .map_err(anyhow::Error::from)
            .and_then(|f| Ok(Decoder::new(BufReader::new(f))?))
        {
            Ok(dec) => {
                let iter = UniformSourceIterator::new(
                    dec,
                    NonZeroU16::new(1).unwrap(),
                    NonZeroU32::new(48000).unwrap(),
                );
                let iter = SampleTypeConverter::new(iter);
                *self.source.borrow_mut() = Some(Box::new(iter));
            }
            Err(e) => {
                eprintln!("Error trying to play {:?}: {e}", &path);
                *self.source.borrow_mut() = None;
            }
        }
        Ok(())
    }
}

impl AudioStream for Jukebox {
    fn write(&self, _channel: usize, samples: &[i16]) -> usize {
        samples.len()
    }

    fn read(&self, buf: &mut [i16]) -> usize {
        if !*self.playing.borrow() {
            for s in buf.iter_mut() {
                *s = 0;
            }
            return buf.len();
        }

        if self.source.borrow().is_none() {
            if self.load_next().is_err() {
                return 0;
            }
        }

        let mut ended = false;
        if let Some(source) = self.source.borrow_mut().as_mut() {
            for out in buf.iter_mut() {
                if let Some(sample) = source.next() {
                    *out = sample;
                } else {
                    ended = true;
                    break;
                }
            }
        }
        if ended {
            *self.source.borrow_mut() = None;
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
pub struct JukeboxInterface {
    music_library: PathBuf,
}

impl JukeboxInterface {
    pub fn new(music_library: PathBuf) -> Self {
        Self { music_library }
    }
}

impl AudioInterface for JukeboxInterface {
    fn create(&self, _client: &HoshiClient, _call: &Call) -> Result<Box<dyn AudioStream>> {
        let stream = Jukebox::new(self.music_library.clone());

        Ok(Box::new(stream))
    }
}
