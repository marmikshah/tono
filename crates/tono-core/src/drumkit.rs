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
    /// Cap on simultaneous hits (oldest stolen with a short fade).
    max_voices: usize,
    /// Steal-declick ramp decrement per sample (≈ 5 ms at the kit's rate).
    fade_step: f32,
}

struct DrumVoice {
    samples: Arc<Vec<f32>>,
    pos: usize,
    gain: f32,
    /// Steal declick: remaining ramp gain (1 = sounding normally) and its
    /// per-sample decrement (0 = not stolen). Culled once silent.
    fade: f32,
    fade_step: f32,
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
            fade_step: 1.0 / (sample_rate as f32 * 0.005),
        }
    }

    /// Strike the drum mapped to `note` (velocity scales its level). Notes with no
    /// mapped piece are ignored.
    pub fn note_on(&mut self, note: Note, velocity: f32) {
        if let Some(samples) = self.pieces.get(&note.midi()) {
            // Steal the oldest still-sounding hit with a ~5 ms fade — a full
            // pool means it is guaranteed audible, so a hard cut would click
            // (a booming 808 kick truncated mid-sample). A hit flood faster
            // than the fade window falls back to hard removal for the bound.
            let sounding = self.voices.iter().filter(|v| v.fade_step == 0.0).count();
            if sounding >= self.max_voices
                && let Some(oldest) = self.voices.iter_mut().find(|v| v.fade_step == 0.0)
            {
                oldest.fade_step = self.fade_step;
            }
            if self.voices.len() >= self.max_voices * 2 {
                self.voices.remove(0);
            }
            self.voices.push(DrumVoice {
                samples: samples.clone(),
                pos: 0,
                gain: velocity.clamp(0.0, 1.0),
                fade: 1.0,
                fade_step: 0.0,
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
                if v.fade_step > 0.0 {
                    v.fade = (v.fade - v.fade_step).max(0.0);
                    if v.fade == 0.0 {
                        v.pos = v.samples.len(); // fully declicked — cull below
                        break;
                    }
                }
                let x = s * v.gain * v.fade;
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

    #[test]
    fn stealing_fades_the_oldest_hit_instead_of_cutting() {
        // Baseline: the kick's own natural sample-to-sample slope.
        let max_jump = |kit: &mut DrumKit, prev: f32| {
            let mut buf = vec![0.0f32; 512 * 2];
            kit.fill(&mut buf);
            let mut m = 0.0f32;
            let mut p = prev;
            for f in 0..512 {
                m = m.max((buf[f * 2] - p).abs());
                p = buf[f * 2];
            }
            m
        };
        let warmup = |kit: &mut DrumKit| {
            kit.note_on(Note(36), 1.0); // kick — long, booming
            let mut out = vec![0.0f32; 64 * 2];
            kit.fill(&mut out);
            out[out.len() - 2]
        };
        let mut natural = DrumKit::general_midi(48_000);
        let prev = warmup(&mut natural);
        let natural_jump = max_jump(&mut natural, prev);

        // Stolen: velocity-0 strikes occupy the pool without adding their own
        // transients, so the mix isolates the stolen kick's ramp-out.
        let mut kit = DrumKit::general_midi(48_000);
        kit.max_voices = 2;
        let prev = warmup(&mut kit);
        kit.note_on(Note(38), 0.0);
        kit.note_on(Note(42), 0.0); // pool full: steals the kick
        let stolen_jump = max_jump(&mut kit, prev);

        // A hard cut steps by the kick's full instantaneous level on top of
        // its natural slope; the 5 ms ramp must add almost nothing.
        assert!(
            stolen_jump <= natural_jump + 0.05,
            "steal clicked: jump {stolen_jump} vs natural {natural_jump}"
        );
        // The stolen voice drains within the declick window.
        kit.fill(&mut vec![0.0f32; 512 * 2]);
        assert_eq!(kit.active_voices(), 2, "faded voice culled after steal");
    }
}
