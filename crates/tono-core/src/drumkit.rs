//! drumkit — a playable drum kit.
//!
//! A pitched instrument maps one sound across the keyboard; a drum kit maps each
//! key to a *different* sound. [`DrumKit`] pre-renders the General MIDI drum map
//! once (via the deterministic `kit` voice) and plays each piece as a one-shot on
//! [`note_on`](DrumKit::note_on) — real-time-safe (a voice is just a cursor into
//! a buffer), so it drops onto a `cpal` callback like any other [`AudioSource`].
//!
//! ```
//! use tono_core::drumkit::DrumKit;
//! use tono_core::instrument::Note;
//!
//! let mut kit = DrumKit::general_midi(48_000);
//! kit.note_on(Note(36), 1.0); // kick
//! kit.note_on(Note(38), 0.8); // snare
//! ```

use std::collections::BTreeMap;
use std::sync::Arc;

use crate::instrument::Note;
use crate::render;
use crate::runtime::AudioSource;

/// General MIDI percussion notes covered by the default kit (kick → ride).
const GM_DRUMS: std::ops::RangeInclusive<u8> = 35..=51;

/// A playable drum kit: MIDI note → a pre-rendered one-shot.
pub struct DrumKit {
    /// note → mono samples (shared so a voice is a cheap cursor).
    pieces: BTreeMap<u8, Arc<Vec<f32>>>,
    voices: Vec<DrumVoice>,
    /// Cap on simultaneous hits (oldest stolen).
    max_voices: usize,
}

struct DrumVoice {
    samples: Arc<Vec<f32>>,
    pos: usize,
    gain: f32,
}

/// Render one GM drum to mono samples via the deterministic `kit` voice, trimmed
/// of trailing silence. Empty if the pitch maps to no drum.
fn render_drum(midi: u8, sample_rate: u32) -> Vec<f32> {
    let doc = serde_json::json!({
        "name": "drum", "duration": 1.5, "sample_rate": sample_rate, "engine": 2,
        "root": { "type": "seq", "bpm": 120, "steps_per_beat": 4, "wave": "kit",
            "env": { "a": 0.001, "d": 0.4, "s": 0.0, "r": 0.15 },
            "notes": [ { "step": 0, "len": 2, "pitch": format!("midi:{midi}"), "gain": 1.0 } ] }
    });
    let Ok(doc) = serde_json::from_value(doc) else {
        return Vec::new();
    };
    let mut s = render::render(&doc);
    let end = s
        .iter()
        .rposition(|x| x.abs() > 1e-4)
        .map(|i| i + 1)
        .unwrap_or(0);
    s.truncate(end);
    s
}

impl DrumKit {
    /// The General MIDI drum map, synthesized once at `sample_rate`.
    pub fn general_midi(sample_rate: u32) -> Self {
        let mut pieces = BTreeMap::new();
        for midi in GM_DRUMS {
            let samples = render_drum(midi, sample_rate);
            if !samples.is_empty() {
                pieces.insert(midi, Arc::new(samples));
            }
        }
        DrumKit {
            pieces,
            voices: Vec::new(),
            max_voices: 32,
        }
    }

    /// Strike the drum mapped to `note` (velocity scales its level). Notes with no
    /// mapped piece are ignored.
    pub fn note_on(&mut self, note: Note, velocity: f32) {
        if let Some(samples) = self.pieces.get(&note.midi()) {
            if self.voices.len() >= self.max_voices {
                self.voices.remove(0);
            }
            self.voices.push(DrumVoice {
                samples: samples.clone(),
                pos: 0,
                gain: velocity.clamp(0.0, 1.0),
            });
        }
    }

    /// The MIDI notes this kit responds to.
    pub fn notes(&self) -> impl Iterator<Item = u8> + '_ {
        self.pieces.keys().copied()
    }

    /// Number of drums still ringing.
    pub fn active_voices(&self) -> usize {
        self.voices.len()
    }
}

impl AudioSource for DrumKit {
    fn fill(&mut self, out: &mut [f32]) -> usize {
        let frames = out.len() / 2;
        out.fill(0.0);
        for v in self.voices.iter_mut() {
            for f in 0..frames {
                let Some(&s) = v.samples.get(v.pos) else {
                    break;
                };
                let x = s * v.gain;
                out[f * 2] += x;
                out[f * 2 + 1] += x;
                v.pos += 1;
            }
        }
        self.voices.retain(|v| v.pos < v.samples.len());
        frames
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn peak(s: &[f32]) -> f32 {
        s.iter().fold(0.0f32, |m, &x| m.max(x.abs()))
    }

    #[test]
    fn gm_kit_maps_and_plays_drums() {
        let mut kit = DrumKit::general_midi(48_000);
        assert!(kit.notes().count() >= 8, "a usable set of GM drums");
        kit.note_on(Note(36), 1.0); // kick
        kit.note_on(Note(38), 0.9); // snare
        assert_eq!(kit.active_voices(), 2);
        let mut out = vec![0.0f32; 512 * 2];
        assert_eq!(kit.fill(&mut out), 512);
        assert!(peak(&out) > 0.0, "the kit makes sound");
        assert!((0..512).all(|f| out[f * 2] == out[f * 2 + 1]), "centered");
    }

    #[test]
    fn one_shots_cull_when_finished() {
        let mut kit = DrumKit::general_midi(48_000);
        kit.note_on(Note(42), 1.0); // closed hi-hat — short
        assert_eq!(kit.active_voices(), 1);
        // Serve well past any drum's length (2 s) so the one-shot ends.
        for _ in 0..200 {
            kit.fill(&mut vec![0.0f32; 512 * 2]);
        }
        assert_eq!(kit.active_voices(), 0, "finished one-shot reclaimed");
    }

    #[test]
    fn unmapped_note_is_ignored() {
        let mut kit = DrumKit::general_midi(48_000);
        kit.note_on(Note(0), 1.0); // nothing at MIDI 0
        assert_eq!(kit.active_voices(), 0);
    }
}
