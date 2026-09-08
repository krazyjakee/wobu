//! RIFF little-endian PCM WAV: mono/stereo, 16-bit, 8–96 kHz; at most ten minutes.
use crate::{MAX_AUDIO_BYTES, Result, invalid};
use serde::{Deserialize, Serialize};
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Info {
    pub channels: u16,
    pub sample_rate: u32,
    pub frames: u64,
    pub duration_ms: u64,
}
pub fn inspect(bytes: &[u8]) -> Result<Info> {
    if bytes.len() < 44
        || bytes.len() > MAX_AUDIO_BYTES
        || &bytes[..4] != b"RIFF"
        || &bytes[8..12] != b"WAVE"
    {
        return Err(invalid("Expected a bounded RIFF PCM WAV file."));
    }
    let u32at = |i| u32::from_le_bytes(bytes[i..i + 4].try_into().expect("checked chunk bounds"));
    if u64::from(u32at(4)) + 8 != bytes.len() as u64 {
        return Err(invalid("WAV RIFF length does not match actual bytes."));
    }
    let mut pos = 12;
    let mut format = None;
    let mut data = None;
    while pos < bytes.len() {
        if bytes.len() - pos < 8 {
            return Err(invalid("Truncated WAV chunk header."));
        }
        let size = u32at(pos + 4) as usize;
        let begin = pos + 8;
        let end = begin
            .checked_add(size)
            .filter(|end| *end <= bytes.len())
            .ok_or_else(|| invalid("Truncated WAV chunk."))?;
        match &bytes[pos..pos + 4] {
            b"fmt " => {
                if format.is_some() || size != 16 {
                    return Err(invalid("Exactly one 16-byte PCM format chunk is supported."));
                }
                let u16at =
                    |i| u16::from_le_bytes(bytes[i..i + 2].try_into().expect("format bounds"));
                let channels = u16at(begin + 2);
                let rate = u32at(begin + 4);
                if u16at(begin) != 1
                    || !(1..=2).contains(&channels)
                    || !(8000..=96000).contains(&rate)
                    || u16at(begin + 14) != 16
                    || u16at(begin + 12) != channels * 2
                    || u32at(begin + 8) != rate * u32::from(channels) * 2
                {
                    return Err(invalid(
                        "WAV must be 16-bit PCM mono/stereo at 8–96 kHz with consistent byte rate.",
                    ));
                }
                format = Some((channels, rate));
            }
            b"data" if data.replace(size).is_some() => {
                return Err(invalid("Duplicate WAV data chunk."));
            }
            _ => {}
        }
        pos = end
            .checked_add(size % 2)
            .filter(|p| *p <= bytes.len())
            .ok_or_else(|| invalid("Missing WAV chunk padding."))?;
    }
    let (channels, sample_rate) = format.ok_or_else(|| invalid("Missing WAV format."))?;
    let size = data.ok_or_else(|| invalid("Missing WAV samples."))?;
    if size == 0 || size % (usize::from(channels) * 2) != 0 {
        return Err(invalid("WAV samples must contain complete nonempty frames."));
    }
    let frames = (size / (usize::from(channels) * 2)) as u64;
    let duration_ms = frames * 1000 / u64::from(sample_rate);
    if duration_ms == 0 || duration_ms > 600_000 {
        return Err(invalid("Audio duration must be 1 ms to ten minutes."));
    }
    Ok(Info { channels, sample_rate, frames, duration_ms })
}
