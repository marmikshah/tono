//! adaptive — reactive music for games.
//!
//! Game music that responds to play: stack **stems** that fade in as the action
//! heats up, drive them with a single `intensity` knob, and fire one-shot
//! **stingers** on events. [`AdaptiveMusic`] is an [`AudioSource`], so it drops
//! onto the same real-time path as everything else and stays deterministic.
//!
//! ```
//! use tono_core::adaptive::{AdaptiveMusic, LoopBuffer};
//! use tono_core::dsl::SoundDoc;
//!
//! # fn demo(drums: &SoundDoc, lead: &SoundDoc) {
//! let mut music = AdaptiveMusic::new(48_000);
//! music.add_layer(LoopBuffer::from_doc(drums), 0.0); // always playing
//! music.add_layer(LoopBuffer::from_doc(lead), 0.6);  // joins when it gets intense
//! music.set_intensity(0.8);                          // combat! the lead swells in
//! # }
//! ```

use crate::dsl::SoundDoc;
use crate::render;
use crate::runtime::AudioSource;

/// A stereo buffer played on a seamless loop — a music stem.
pub struct LoopBuffer {
    left: Vec<f32>,
    right: Vec<f32>,
    pos: usize,
}

impl LoopBuffer {
    /// Render `doc` once and loop it. (For a click-free loop, author the doc with
    /// a `loop` playback so its tail meets its head, or trim it with
    /// [`from_stereo`](Self::from_stereo).)
    pub fn from_doc(doc: &SoundDoc) -> Self {
        let p = render::render_product(doc);
        let (left, right) = p.stereo.unwrap_or_else(|| (p.mono.clone(), p.mono));
        LoopBuffer::from_stereo(left, right)
    }

    /// Loop pre-rendered stereo samples — trim them to a musical bar length for a
    /// seamless loop. Channels of unequal length are truncated to the shorter
    /// one: indexing past the short channel would panic on the audio thread.
    pub fn from_stereo(mut left: Vec<f32>, mut right: Vec<f32>) -> Self {
        let n = left.len().min(right.len());
        left.truncate(n);
        right.truncate(n);
        LoopBuffer {
            left,
            right,
            pos: 0,
        }
    }
}

impl AudioSource for LoopBuffer {
    fn fill(&mut self, out: &mut [f32]) -> usize {
        let frames = out.len() / 2;
        let n = self.left.len();
        if n == 0 {
            out.fill(0.0);
            return frames;
        }
        for f in 0..frames {
            out[f * 2] = self.left[self.pos % n];
            out[f * 2 + 1] = self.right[self.pos % n];
            self.pos += 1;
        }
        frames
    }

    fn reset(&mut self) {
        self.pos = 0;
    }
}

struct Layer {
    source: Box<dyn AudioSource + Send>,
    /// Intensity at/above which this stem plays at full volume.
    fade_in_at: f32,
    gain: f32,
    target: f32,
}

struct Stinger {
    left: Vec<f32>,
    right: Vec<f32>,
    pos: usize,
}

/// A layered, intensity-driven music bed with one-shot stingers.
pub struct AdaptiveMusic {
    layers: Vec<Layer>,
    stingers: Vec<Stinger>,
    intensity: f32,
    /// Per-sample one-pole coefficient for the layer cross-fades.
    fade_coeff: f32,
    scratch: Vec<f32>,
    sample_rate: u32,
    /// Frozen: `fill` outputs silence and holds the position clock + layers.
    paused: bool,
    /// Frames rendered while playing since construction or the last `reset` —
    /// the musical clock a beat-locked game derives its beat position from.
    position: u64,
    /// Current master duck multiplier (1.0 = no duck), recovering per sample.
    duck_gain: f32,
    /// Per-sample one-pole coefficient for the duck recovery.
    duck_coeff: f32,
}

impl AdaptiveMusic {
    /// A new, empty bed. Layer cross-fades take ~1.5 s.
    pub fn new(sample_rate: u32) -> Self {
        let fade_coeff = 1.0 - (-1.0 / (1.5 * sample_rate as f32)).exp();
        AdaptiveMusic {
            layers: Vec::new(),
            stingers: Vec::new(),
            intensity: 0.0,
            fade_coeff,
            scratch: Vec::new(),
            sample_rate,
            paused: false,
            position: 0,
            duck_gain: 1.0,
            duck_coeff: 0.0,
        }
    }

    /// Add a stem that fades to full volume once `intensity >= fade_in_at`
    /// (`0.0` = always on). It plays silently underneath until then.
    pub fn add_layer(&mut self, source: impl AudioSource + Send + 'static, fade_in_at: f32) {
        let on = self.intensity >= fade_in_at;
        self.layers.push(Layer {
            source: Box::new(source),
            fade_in_at,
            gain: if on { 1.0 } else { 0.0 },
            target: if on { 1.0 } else { 0.0 },
        });
    }

    /// Set the intensity, 0..1 — stems cross-fade toward their new levels.
    pub fn set_intensity(&mut self, x: f32) {
        self.intensity = x.clamp(0.0, 1.0);
        for l in &mut self.layers {
            l.target = if self.intensity >= l.fade_in_at {
                1.0
            } else {
                0.0
            };
        }
    }

    /// Fire a one-shot stinger over the bed (rendered now, mixed until it ends).
    pub fn stinger(&mut self, doc: &SoundDoc) {
        let p = render::render_product(doc);
        let (left, right) = p.stereo.unwrap_or_else(|| (p.mono.clone(), p.mono));
        self.stingers.push(Stinger {
            left,
            right,
            pos: 0,
        });
    }

    /// The current intensity.
    pub fn intensity(&self) -> f32 {
        self.intensity
    }

    /// A layer's current (cross-faded) gain — for a mixer readout.
    pub fn layer_gain(&self, index: usize) -> Option<f32> {
        self.layers.get(index).map(|l| l.gain)
    }

    /// Number of stingers still ringing.
    pub fn active_stingers(&self) -> usize {
        self.stingers.len()
    }

    // ---- Transport ----

    /// Freeze the bed: [`fill`](AudioSource::fill) outputs silence and both the
    /// position clock and every layer hold their place, so [`resume`](Self::resume)
    /// continues seamlessly.
    pub fn pause(&mut self) {
        self.paused = true;
    }

    /// Resume from a [`pause`](Self::pause).
    pub fn resume(&mut self) {
        self.paused = false;
    }

    /// Whether the bed is currently paused.
    pub fn is_paused(&self) -> bool {
        self.paused
    }

    /// Restart the bed from the top: the position clock returns to 0, every
    /// layer rewinds to its loop head, and ringing stingers are cleared. The
    /// intensity and layer target gains are left as they are — call this to line
    /// the music up with a beat clock at sample 0.
    pub fn reset(&mut self) {
        self.position = 0;
        self.stingers.clear();
        for l in &mut self.layers {
            l.source.reset();
        }
    }

    /// Frames rendered while playing since construction or the last
    /// [`reset`](Self::reset) — the musical clock. Beats derive from it exactly
    /// (`beats = position / (sample_rate * 60 / bpm)`), so a game's beat-locked
    /// schedule stays phase-aligned with what is sounding. Holds while paused.
    pub fn position_frames(&self) -> u64 {
        self.position
    }

    /// Duck the whole bed now — drop its gain to `1.0 - depth` and recover to
    /// unity over `release`. A fast sidechain for stingers or SFX, independent of
    /// the (slower) intensity cross-fade. `depth` clamps to `0..=1`; a stinger
    /// duck is typically shallow and short (e.g. `0.4`, ~180 ms).
    pub fn duck(&mut self, depth: f32, release: std::time::Duration) {
        self.duck_gain = (1.0 - depth).clamp(0.0, 1.0);
        let secs = release.as_secs_f32().max(1.0 / self.sample_rate as f32);
        self.duck_coeff = 1.0 - (-1.0 / (secs * self.sample_rate as f32)).exp();
    }
}

impl AudioSource for AdaptiveMusic {
    fn fill(&mut self, out: &mut [f32]) -> usize {
        let frames = out.len() / 2;
        out.fill(0.0);
        // Paused: silence, and hold the clock + every layer where they are.
        if self.paused {
            return frames;
        }
        if self.scratch.len() < frames * 2 {
            self.scratch.resize(frames * 2, 0.0);
        }
        let coeff = self.fade_coeff;
        let scratch = &mut self.scratch[..frames * 2];
        for layer in &mut self.layers {
            layer.source.fill(scratch);
            for f in 0..frames {
                layer.gain += (layer.target - layer.gain) * coeff;
                out[f * 2] += scratch[f * 2] * layer.gain;
                out[f * 2 + 1] += scratch[f * 2 + 1] * layer.gain;
            }
        }
        for st in &mut self.stingers {
            for f in 0..frames {
                let (Some(&l), Some(&r)) = (st.left.get(st.pos), st.right.get(st.pos)) else {
                    break;
                };
                out[f * 2] += l;
                out[f * 2 + 1] += r;
                st.pos += 1;
            }
        }
        self.stingers.retain(|s| s.pos < s.left.len());
        // Master duck recovery (skip entirely when at unity and idle).
        if self.duck_gain < 1.0 {
            for f in 0..frames {
                self.duck_gain += (1.0 - self.duck_gain) * self.duck_coeff;
                out[f * 2] *= self.duck_gain;
                out[f * 2 + 1] *= self.duck_gain;
            }
        }
        self.position += frames as u64;
        frames
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn doc(json: &str) -> SoundDoc {
        serde_json::from_str(json).unwrap()
    }
    fn peak(s: &[f32]) -> f32 {
        s.iter().fold(0.0f32, |m, &x| m.max(x.abs()))
    }

    #[test]
    fn layers_fade_with_intensity() {
        let base = doc(r#"{ "name":"b", "duration":0.2, "root": { "type":"sine", "freq":220 } }"#);
        let hi = doc(r#"{ "name":"h", "duration":0.2, "root": { "type":"sine", "freq":880 } }"#);
        let mut music = AdaptiveMusic::new(48_000);
        music.add_layer(LoopBuffer::from_doc(&base), 0.0); // always on
        music.add_layer(LoopBuffer::from_doc(&hi), 0.5); // fades in at 0.5

        let mut out = vec![0.0f32; 512 * 2];
        music.fill(&mut out);
        assert!(peak(&out) > 0.0, "base layer sounds");
        assert_eq!(
            music.layer_gain(1),
            Some(0.0),
            "hi layer silent at intensity 0"
        );

        music.set_intensity(1.0);
        for _ in 0..400 {
            music.fill(&mut vec![0.0f32; 512 * 2]); // let the cross-fade run
        }
        assert!(
            music.layer_gain(1).unwrap() > 0.9,
            "hi layer swelled in with intensity"
        );
    }

    #[test]
    fn stingers_fire_and_finish() {
        let base = doc(r#"{ "name":"b", "duration":0.2, "root": { "type":"sine", "freq":220 } }"#);
        let sting =
            doc(r#"{ "name":"s", "duration":0.05, "root": { "type":"sine", "freq":1320 } }"#);
        let mut music = AdaptiveMusic::new(48_000);
        music.add_layer(LoopBuffer::from_doc(&base), 0.0);
        music.stinger(&sting);
        assert_eq!(music.active_stingers(), 1);
        // 0.05 s ≈ 2400 frames; serve well past it.
        for _ in 0..20 {
            music.fill(&mut vec![0.0f32; 512 * 2]);
        }
        assert_eq!(
            music.active_stingers(),
            0,
            "stinger finished and was culled"
        );
    }

    #[test]
    fn loop_buffer_reset_rewinds_to_head() {
        let base = doc(r#"{ "name":"b", "duration":0.2, "root": { "type":"sine", "freq":220 } }"#);
        let mut buf = LoopBuffer::from_doc(&base);

        let mut first = vec![0.0f32; 64 * 2];
        buf.fill(&mut first);
        buf.fill(&mut vec![0.0f32; 512 * 2]); // advance somewhere else in the loop
        buf.reset();
        let mut again = vec![0.0f32; 64 * 2];
        buf.fill(&mut again);

        assert_eq!(
            first, again,
            "reset replays from the loop head, sample-identical"
        );
    }

    #[test]
    fn position_advances_while_playing_and_holds_when_paused() {
        let base = doc(r#"{ "name":"b", "duration":0.2, "root": { "type":"sine", "freq":220 } }"#);
        let mut music = AdaptiveMusic::new(48_000);
        music.add_layer(LoopBuffer::from_doc(&base), 0.0);

        assert_eq!(music.position_frames(), 0);
        music.fill(&mut vec![0.0f32; 512 * 2]);
        assert_eq!(music.position_frames(), 512, "the clock advances by frames");

        music.pause();
        assert!(music.is_paused());
        let mut out = vec![0.1f32; 512 * 2];
        music.fill(&mut out);
        assert_eq!(peak(&out), 0.0, "paused output is silent");
        assert_eq!(music.position_frames(), 512, "the clock holds while paused");

        music.resume();
        music.fill(&mut vec![0.0f32; 512 * 2]);
        assert_eq!(music.position_frames(), 1024, "resumes advancing");
    }

    #[test]
    fn reset_zeroes_the_clock_and_restarts_layers() {
        let base = doc(r#"{ "name":"b", "duration":0.2, "root": { "type":"sine", "freq":220 } }"#);
        let mut music = AdaptiveMusic::new(48_000);
        music.add_layer(LoopBuffer::from_doc(&base), 0.0);
        music.set_intensity(1.0);

        music.fill(&mut vec![0.0f32; 512 * 2]);
        assert!(music.position_frames() > 0);
        music.reset();
        assert_eq!(music.position_frames(), 0, "the clock is back at sample 0");

        let mut out = vec![0.0f32; 512 * 2];
        music.fill(&mut out);
        assert!(peak(&out) > 0.0, "the bed plays again from the top");
    }

    #[test]
    fn duck_attenuates_then_recovers() {
        use std::time::Duration;
        let base = doc(r#"{ "name":"b", "duration":0.2, "root": { "type":"sine", "freq":220 } }"#);

        // Undicked reference peak over one block.
        let mut plain = AdaptiveMusic::new(48_000);
        plain.add_layer(LoopBuffer::from_doc(&base), 0.0);
        let mut plain_out = vec![0.0f32; 512 * 2];
        plain.fill(&mut plain_out);
        let reference = peak(&plain_out);

        // A fresh, identical bed, ducked hard just before the same block.
        let mut music = AdaptiveMusic::new(48_000);
        music.add_layer(LoopBuffer::from_doc(&base), 0.0);
        music.duck(0.9, Duration::from_millis(180));
        let mut ducked = vec![0.0f32; 512 * 2];
        music.fill(&mut ducked);
        assert!(
            peak(&ducked) < reference,
            "ducked block is quieter than the undicked reference"
        );

        // Recover well past the release, then the gain is back near unity.
        for _ in 0..64 {
            music.fill(&mut vec![0.0f32; 512 * 2]);
        }
        let mut recovered = vec![0.0f32; 512 * 2];
        music.fill(&mut recovered);
        assert!(
            peak(&recovered) > 0.9 * reference,
            "the duck recovered toward unity"
        );
    }
}
