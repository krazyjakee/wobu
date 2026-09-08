//! Millisecond intervals. Viseme names are neutral labels; adapters map them explicitly.
use crate::{MAX_TIMING_BYTES, Result, invalid};
use serde::{Deserialize, Serialize};
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    Word,
    Phoneme,
    Viseme,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Cue {
    pub start_ms: u64,
    pub end_ms: u64,
    pub kind: Kind,
    pub value: String,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Track {
    pub version: u32,
    pub audio_hash: String,
    pub duration_ms: u64,
    pub cues: Vec<Cue>,
}
impl Track {
    pub fn validate(&self, audio_hash: &str, duration_ms: u64) -> Result<()> {
        if self.version != 1
            || self.audio_hash != audio_hash
            || self.duration_ms != duration_ms
            || self.cues.len() > 10_000
        {
            return Err(invalid(
                "Timing version, audio hash or duration mismatch, or too many cues.",
            ));
        }
        let mut last = [0; 3];
        for cue in &self.cues {
            let index = match cue.kind {
                Kind::Word => 0,
                Kind::Phoneme => 1,
                Kind::Viseme => 2,
            };
            if cue.start_ms < last[index]
                || cue.start_ms >= cue.end_ms
                || cue.end_ms > duration_ms
                || cue.value.is_empty()
                || cue.value.len() > 256
            {
                return Err(invalid(
                    "Timing cues must be ordered, non-overlapping within each kind and inside the audio duration.",
                ));
            }
            if cue.kind == Kind::Viseme
                && ![
                    "sil", "PP", "FF", "TH", "DD", "kk", "CH", "SS", "nn", "RR", "aa", "E", "ih",
                    "oh", "ou",
                ]
                .contains(&cue.value.as_str())
            {
                return Err(invalid("Unknown neutral viseme label."));
            }
            last[index] = cue.end_ms;
        }
        Ok(())
    }
    pub fn active(&self, ms: u64) -> impl Iterator<Item = &Cue> {
        self.cues.iter().filter(move |c| c.start_ms <= ms && ms < c.end_ms)
    }
}
pub fn decode(bytes: &[u8], hash: &str, duration: u64) -> Result<Track> {
    if bytes.len() > MAX_TIMING_BYTES {
        return Err(invalid("Timing exceeds 2 MiB."));
    }
    let track: Track = serde_json::from_slice(bytes).map_err(|e| invalid(e.to_string()))?;
    track.validate(hash, duration)?;
    Ok(track)
}
