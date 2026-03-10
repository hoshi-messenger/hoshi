use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum AudioChunk {
    ULaw {
        id: String,
        chunk_offset: i32,
        sample_rate: u16,
        /// μ-law encoded PCM, one byte per sample
        samples: Vec<u8>,
    },
}

impl AudioChunk {
    pub fn id(&self) -> &str {
        match self {
            Self::ULaw { id, .. } => id,
        }
    }

    pub fn chunk_offset(&self) -> i32 {
        match self {
            Self::ULaw { chunk_offset, .. } => *chunk_offset,
        }
    }

    pub fn sample_rate(&self) -> u16 {
        match self {
            Self::ULaw { sample_rate, .. } => *sample_rate,
        }
    }

    pub fn samples(&self) -> &[u8] {
        match self {
            Self::ULaw { samples, .. } => samples,
        }
    }

    pub fn decode_i16(&self) -> Vec<i16> {
        self.samples().iter().map(|&b| ulaw_to_linear(b)).collect()
    }
}

fn ulaw_to_linear(u_val: u8) -> i16 {
    let u = !u_val;
    let t = (((u & 0x0F) as i32) << 3) + 0x84;
    let t = t << ((u & 0x70) >> 4);
    // u & 0x80 != 0 means the original sign bit was 1 (positive)
    if u & 0x80 != 0 {
        (t - 0x84) as i16
    } else {
        (0x84 - t) as i16
    }
}

pub(crate) fn linear_to_ulaw(sample: i16) -> u8 {
    const BIAS: i32 = 0x84;
    const CLIP: i32 = 32635;

    let sign = if sample >= 0 { 0x80u8 } else { 0x00u8 };
    let mag = (if sample >= 0 {
        sample as i32
    } else {
        -(sample as i32)
    })
    .min(CLIP)
        + BIAS;

    let leading = (mag as u32).leading_zeros();
    let highest_bit = 31u32.saturating_sub(leading);
    let exp = (highest_bit.saturating_sub(7) as u8).min(7);
    let mantissa = ((mag >> (exp + 3)) & 0x0F) as u8;

    !(sign | (exp << 4) | mantissa)
}
