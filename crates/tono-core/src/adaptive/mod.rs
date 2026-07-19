//! adaptive — reactive music for games.
//!
//! Game music that responds to play: stack **stems** that fade in as the action
//! heats up, drive them with a single `intensity` knob, and fire one-shot
//! **stingers** on events. [`AdaptiveMusic`] is an [`AudioSource`], so it drops
//! onto the same real-time path as everything else and stays deterministic.
//!
//! ```
//! use tono_core::adaptive::AdaptiveMusic;
//! use tono_core::dsl::SoundDoc;
//!
//! # fn demo(drums: &SoundDoc, lead: &SoundDoc) {
//! let mut music = AdaptiveMusic::new(48_000);
//! music.add_layer_doc(drums, 0.0); // always playing (rendered at 48 kHz)
//! music.add_layer_doc(lead, 0.6);  // joins when it gets intense
//! music.set_intensity(0.8);        // combat! the lead swells in
//! # }
//! ```

mod layers;
mod schedule;
mod sections;

use crate::dsl::SoundDoc;
use crate::render;
use crate::runtime::AudioSource;
pub use schedule::Quantize;

/// A stereo buffer played on a seamless loop — a music stem.
pub struct LoopBuffer {
    left: Vec<f32>,
    right: Vec<f32>,
    pos: usize,
}

impl LoopBuffer {
    /// Render `doc` once **at the doc's own `sample_rate`** and loop it. If
    /// the buffer will play through an [`AdaptiveMusic`]/engine running at a
    /// different rate, use [`from_doc_at`](Self::from_doc_at) — a rate
    /// mismatch plays back silently detuned. (For a click-free loop, author
    /// the doc with a `loop` playback so its tail meets its head, or trim it
    /// with [`from_stereo`](Self::from_stereo).)
    pub fn from_doc(doc: &SoundDoc) -> Self {
        let (left, right) = render_stereo_pair(doc);
        LoopBuffer::from_stereo(left, right)
    }

    /// [`from_doc`](Self::from_doc) rendered at an explicit `sample_rate` —
    /// the safe constructor when the playback rate is the engine's, not the
    /// doc's.
    pub fn from_doc_at(doc: &SoundDoc, sample_rate: u32) -> Self {
        LoopBuffer::from_doc(&doc_at(doc, sample_rate))
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

    /// Render `doc` and loop it at **exactly** `frames` samples — truncated or
    /// zero-padded to that length. Use it to force a set of stems onto one shared
    /// loop grid so their layered cross-fades stay sample-aligned (never drift
    /// phase). See [`AdaptiveMusic::add_stem_set`].
    pub fn from_doc_len(doc: &SoundDoc, frames: usize) -> Self {
        let (mut left, mut right) = render_stereo_pair(doc);
        left.resize(frames, 0.0);
        right.resize(frames, 0.0);
        LoopBuffer {
            left,
            right,
            pos: 0,
        }
    }

    /// [`from_doc_len`](Self::from_doc_len) rendered at an explicit
    /// `sample_rate` (see [`from_doc_at`](Self::from_doc_at)).
    pub fn from_doc_len_at(doc: &SoundDoc, frames: usize, sample_rate: u32) -> Self {
        LoopBuffer::from_doc_len(&doc_at(doc, sample_rate), frames)
    }
}

/// The doc re-stamped to render at `sample_rate` — how every doc-taking
/// [`AdaptiveMusic`] entry point pins rendering to the engine's rate (the same
/// override `Engine::new_player` applies), so a doc left at the default
/// 44 100 Hz can't play detuned through a 48 kHz engine.
fn doc_at(doc: &SoundDoc, sample_rate: u32) -> SoundDoc {
    let mut doc = doc.clone();
    doc.sample_rate = sample_rate;
    doc
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
            out[f * 2] = self.left[self.pos];
            out[f * 2 + 1] = self.right[self.pos];
            // Wrap eagerly so `pos` stays in `0..n` — no modulo on the hot
            // loop, and no usize overflow on very long sessions.
            self.pos += 1;
            if self.pos == n {
                self.pos = 0;
            }
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

impl Stinger {
    /// Channels of unequal length are truncated to the shorter one, so `fill`
    /// can index both directly and the spent-stinger cull is exact.
    fn new(mut left: Vec<f32>, mut right: Vec<f32>) -> Self {
        let n = left.len().min(right.len());
        left.truncate(n);
        right.truncate(n);
        Stinger {
            left,
            right,
            pos: 0,
        }
    }
}

/// Duck attack time — a couple of ms is fast enough to feel instant without
/// the click a gain step would make.
const DUCK_ATTACK_SECS: f32 = 0.002;
/// Snap threshold (~-80 dB from target) so the exponential ramps land exactly
/// instead of asymptoting forever.
const DUCK_SNAP_EPSILON: f32 = 1e-4;

/// Render a doc once and return its stereo pair — mono is **duplicated**, the
/// doc's Haas/Wide `stereo` treatment is NOT applied (unlike
/// `player::render_stereo`, which stereoizes; unifying the two would change
/// adaptive playback bytes for treated docs, so the divergence is deliberate).
/// Public so hosts can render off the audio thread and hand the buffers to
/// [`AdaptiveMusic::stinger_stereo`].
pub fn render_stereo_pair(doc: &SoundDoc) -> (Vec<f32>, Vec<f32>) {
    let p = render::render_product(doc);
    p.stereo.unwrap_or_else(|| (p.mono.clone(), p.mono))
}

/// A deferred change fired when the clock reaches `fire_at` frames.
struct Scheduled {
    fire_at: u64,
    action: Action,
}

enum Action {
    SetIntensity(f32),
    Stinger(Stinger),
    Transition { to: usize },
}

/// A horizontal section: one looping bed swapped in on a beat boundary.
struct Section {
    name: String,
    buffer: LoopBuffer,
}

/// A short declick cross-fade from the current section to `to`.
struct SectionFade {
    to: usize,
    /// The gain the fade started from (0.0 entering; the mid-fade value when
    /// a reversal flips the step).
    from_gain: f32,
    /// Per-frame gain increment; negative when a cancelled transition ramps
    /// the fade back down.
    step: f32,
    /// Frames faded so far. The gain is computed from this absolute count
    /// (`from_gain + frames_done * step`), never accumulated per span, so the
    /// ramp is bit-exact for any host block size.
    frames_done: usize,
}

/// A layered, intensity-driven music bed with one-shot stingers.
///
/// Horizontal re-sequencing — sections that swap on the bar, like a film
/// score reacting to play:
///
/// ```
/// use tono_core::adaptive::{AdaptiveMusic, Quantize};
/// use tono_core::dsl::{Node, SoundDoc};
///
/// let explore = SoundDoc::new("explore", Node::Sine { freq: 220.0.into() });
/// let battle = SoundDoc::new("battle", Node::Sine { freq: 261.6.into() });
///
/// let mut music = AdaptiveMusic::new(48_000);
/// music.set_tempo(120.0, 4);                    // beats drive the boundaries
/// music.add_section("explore", &explore);       // starts playing
/// let battle = music.add_section("battle", &battle);
///
/// music.transition_to(battle, Quantize::Bar);   // combat! swaps on the next bar
/// music.set_intensity_at(0.9, Quantize::Beat);  // stems swell on the beat
/// ```
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
    /// Current master duck multiplier (1.0 = no duck), moving per sample.
    duck_gain: f32,
    /// The floor the current duck is attacking toward; snaps back to 1.0 once
    /// reached so the release phase takes over.
    duck_target: f32,
    /// Per-sample one-pole coefficient for the (fast) duck attack.
    duck_attack: f32,
    /// Per-sample one-pole coefficient for the duck recovery.
    duck_coeff: f32,
    /// Tempo for beat/bar quantization (`0` = unset → everything is immediate).
    bpm: f32,
    /// Beats per bar (time-signature numerator) for bar quantization.
    beats_per_bar: u32,
    /// Deferred changes fired when the clock reaches their frame.
    pending: Vec<Scheduled>,
    /// Horizontal sections; the current one plays as the base bed.
    sections: Vec<Section>,
    /// Index of the section currently sounding (`None` until one is added).
    current_section: Option<usize>,
    /// An in-flight section cross-fade.
    section_fade: Option<SectionFade>,
    /// Per-sample increment for a section cross-fade (~60 ms declick swap).
    section_step: f32,
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
            duck_target: 1.0,
            duck_attack: 0.0,
            duck_coeff: 0.0,
            bpm: 0.0,
            beats_per_bar: 4,
            pending: Vec::new(),
            sections: Vec::new(),
            current_section: None,
            section_fade: None,
            // A ~60 ms declick cross-fade — the swap is already beat-aligned, so
            // this only smooths the seam, it doesn't blur the downbeat.
            section_step: 1.0 / (0.06 * sample_rate as f32),
        }
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
    /// layer rewinds to its loop head, ringing stingers are cleared, any pending
    /// quantized schedules are dropped, and an in-flight duck resets to unity.
    /// The intensity and layer target gains are left as they are — call this to
    /// line the music up with a beat clock at sample 0.
    pub fn reset(&mut self) {
        self.position = 0;
        self.stingers.clear();
        self.pending.clear();
        self.section_fade = None;
        // Any in-flight duck is dropped — a restart starts at unity.
        self.duck_gain = 1.0;
        self.duck_target = 1.0;
        for l in &mut self.layers {
            l.source.reset();
        }
        for s in &mut self.sections {
            s.buffer.reset();
        }
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
        // Apply each scheduled action at its exact frame — render sub-spans
        // around boundaries, so the output is identical for any host block
        // size (an action must not fire early just because the block is big).
        let mut done = 0usize;
        while done < frames {
            self.fire_due();
            let next = self
                .pending
                .iter()
                .map(|s| s.fire_at)
                .filter(|&t| t > self.position)
                .min();
            let span = match next {
                Some(t) => ((t - self.position) as usize).min(frames - done),
                None => frames - done,
            };
            self.render_span(&mut out[done * 2..(done + span) * 2], span);
            done += span;
        }
        frames
    }

    /// Restart the bed through the trait as well — previously only the
    /// inherent [`reset`](AdaptiveMusic::reset) ran, so via
    /// `&mut dyn AudioSource` (a host transport, a mixer) reset was a silent
    /// no-op that left the clock, stingers, and schedules running.
    fn reset(&mut self) {
        AdaptiveMusic::reset(self);
    }
}

impl AdaptiveMusic {
    /// Render one uninterrupted span of `frames` frames into `out` (sections,
    /// layers, stingers, duck) and advance the clock. [`AudioSource::fill`]
    /// splits the block at schedule boundaries and calls this per sub-span.
    fn render_span(&mut self, out: &mut [f32], frames: usize) {
        if self.scratch.len() < frames * 2 {
            self.scratch.resize(frames * 2, 0.0);
        }
        let coeff = self.fade_coeff;
        let scratch = &mut self.scratch[..frames * 2];

        // Horizontal section (the base bed), optionally cross-fading to a new one.
        // The fade's step can be negative: a cancelled transition ramps the same
        // fade back down to 0 (a click-free reversal) instead of completing.
        let fade = self
            .section_fade
            .as_ref()
            .map(|f| (f.to, f.from_gain, f.step, f.frames_done));
        if let Some((to, start, step, done)) = fade {
            let g = |f: usize| (start + (done + f) as f32 * step).clamp(0.0, 1.0);
            if let Some(cur) = self.current_section {
                self.sections[cur].buffer.fill(scratch);
                for f in 0..frames {
                    let g = g(f);
                    out[f * 2] += scratch[f * 2] * (1.0 - g);
                    out[f * 2 + 1] += scratch[f * 2 + 1] * (1.0 - g);
                }
            }
            self.sections[to].buffer.fill(scratch);
            for f in 0..frames {
                let g = g(f);
                out[f * 2] += scratch[f * 2] * g;
                out[f * 2 + 1] += scratch[f * 2 + 1] * g;
            }
            let end = g(frames);
            if end >= 1.0 {
                self.current_section = Some(to);
                self.section_fade = None;
            } else if end <= 0.0 {
                // A reversed fade settled back on the current section.
                self.section_fade = None;
            } else if let Some(fd) = self.section_fade.as_mut() {
                fd.frames_done = done + frames;
            }
        } else if let Some(cur) = self.current_section {
            self.sections[cur].buffer.fill(scratch);
            for f in 0..frames {
                out[f * 2] += scratch[f * 2];
                out[f * 2 + 1] += scratch[f * 2 + 1];
            }
        }

        for layer in &mut self.layers {
            layer.source.fill(scratch);
            for f in 0..frames {
                layer.gain += (layer.target - layer.gain) * coeff;
                // Snap at the fade's end (like the duck below) so a faded-out
                // layer reaches exactly 0 — the one-pole otherwise asymptotes
                // forever, rendering inaudible content at full CPU cost.
                if (layer.target - layer.gain).abs() < DUCK_SNAP_EPSILON {
                    layer.gain = layer.target;
                }
                out[f * 2] += scratch[f * 2] * layer.gain;
                out[f * 2 + 1] += scratch[f * 2 + 1] * layer.gain;
            }
        }
        for st in &mut self.stingers {
            // Channels are equal length by construction (`Stinger::new`), so
            // direct indexing is safe and the cull below is exact.
            let n = (st.left.len() - st.pos).min(frames);
            for f in 0..n {
                out[f * 2] += st.left[st.pos];
                out[f * 2 + 1] += st.right[st.pos];
                st.pos += 1;
            }
        }
        self.stingers.retain(|s| s.pos < s.left.len());
        // Master duck: attack toward the floor, then release to unity. Skip
        // entirely when idle at unity so the common case pays nothing.
        if self.duck_gain < 1.0 || self.duck_target < 1.0 {
            for f in 0..frames {
                if self.duck_gain > self.duck_target {
                    // Attack: ramp down to the floor over a few ms (no click).
                    self.duck_gain += (self.duck_target - self.duck_gain) * self.duck_attack;
                    if self.duck_gain <= self.duck_target + DUCK_SNAP_EPSILON {
                        self.duck_gain = self.duck_target;
                        // Floor reached — release from here on.
                        self.duck_target = 1.0;
                    }
                } else {
                    // Release: recover toward unity, snapping so it doesn't
                    // asymptote below 1.0 forever (a permanent tiny attenuation).
                    self.duck_gain += (1.0 - self.duck_gain) * self.duck_coeff;
                    if self.duck_gain >= 1.0 - DUCK_SNAP_EPSILON {
                        self.duck_gain = 1.0;
                    }
                }
                out[f * 2] *= self.duck_gain;
                out[f * 2 + 1] *= self.duck_gain;
            }
        }
        self.position += frames as u64;
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

    // ---- interactive-music v2 ----

    fn tone(freq: f32) -> SoundDoc {
        doc(&format!(
            r#"{{ "name":"t", "duration":0.25, "root": {{ "type":"sine", "freq":{freq} }} }}"#
        ))
    }

    /// Advance the clock by exactly `frames` in one block.
    fn advance(m: &mut AdaptiveMusic, frames: usize) {
        m.fill(&mut vec![0.0f32; frames * 2]);
    }

    #[test]
    fn clock_counts_beats_and_bars() {
        let mut m = AdaptiveMusic::new(48_000);
        m.add_layer(LoopBuffer::from_doc(&tone(220.0)), 0.0);
        m.set_tempo(120.0, 4); // 120 bpm → 24 000 frames/beat at 48 kHz
        advance(&mut m, 24_000);
        assert!(
            (m.beats() - 1.0).abs() < 1e-6,
            "one beat elapsed: {}",
            m.beats()
        );
        advance(&mut m, 24_000 * 7); // to 8 beats = 2 bars total
        assert!(
            (m.bars() - 2.0).abs() < 1e-6,
            "two bars elapsed: {}",
            m.bars()
        );
    }

    #[test]
    fn intensity_change_waits_for_the_bar() {
        let mut m = AdaptiveMusic::new(48_000);
        m.add_layer(LoopBuffer::from_doc(&tone(220.0)), 0.0);
        m.add_layer(LoopBuffer::from_doc(&tone(880.0)), 0.5); // joins at intensity ≥ 0.5
        m.set_tempo(120.0, 4); // 96 000 frames/bar
        m.set_intensity_at(1.0, Quantize::Bar);
        // A block short of the bar: the target has not been raised yet.
        advance(&mut m, 48_000);
        assert_eq!(m.intensity(), 0.0, "intensity holds before the bar");
        // Cross the bar line: the scheduled change fires.
        advance(&mut m, 48_100);
        assert_eq!(m.intensity(), 1.0, "intensity applied on the bar");
    }

    #[test]
    fn transition_swaps_sections_on_the_bar() {
        let mut m = AdaptiveMusic::new(48_000);
        let explore = m.add_section("explore", &tone(330.0));
        let battle = m.add_section("battle", &tone(660.0));
        assert_eq!(m.current_section(), Some(explore));
        assert_eq!(m.section_named("battle"), Some(battle));
        m.set_tempo(120.0, 4); // 96 000 frames/bar
        m.transition_to(battle, Quantize::Bar);
        // Before the bar: still on explore, and audible.
        let mut out = vec![0.0f32; 512 * 2];
        m.fill(&mut out);
        assert!(peak(&out) > 0.0, "section audio plays");
        assert_eq!(m.current_section(), Some(explore), "holds until the bar");
        // Past the bar + the short cross-fade: now on battle.
        for _ in 0..200 {
            advance(&mut m, 512);
        }
        assert_eq!(m.current_section(), Some(battle), "swapped to battle");
    }

    #[test]
    fn buffer_and_doc_section_apis_agree() {
        // add_section_buffer (off-lock render) must match add_section (renders
        // internally) sample-for-sample. add_section renders at the ENGINE's
        // rate, so the off-lock caller uses from_doc_at with the same rate.
        let d = tone(220.0);
        let via_doc = {
            let mut m = AdaptiveMusic::new(48_000);
            m.add_section("a", &d);
            let mut o = vec![0.0f32; 256 * 2];
            m.fill(&mut o);
            o
        };
        let via_buf = {
            let mut m = AdaptiveMusic::new(48_000);
            m.add_section_buffer("a", LoopBuffer::from_doc_at(&d, 48_000));
            let mut o = vec![0.0f32; 256 * 2];
            m.fill(&mut o);
            o
        };
        assert_eq!(
            via_doc, via_buf,
            "add_section_buffer must match add_section"
        );
    }

    #[test]
    fn doc_apis_render_at_the_engine_rate() {
        // A doc left at the default 44_100 must render at the ENGINE's rate
        // through every doc-taking entry point — the old behavior played it
        // detuned ~9% through a 48 kHz engine.
        let d = tone(220.0); // tone() leaves doc.sample_rate at the default
        assert_ne!(d.sample_rate, 48_000, "test needs a mismatched doc");

        let mut at_engine_rate = AdaptiveMusic::new(48_000);
        at_engine_rate.add_layer_doc(&d, 0.0);
        let mut a = vec![0.0f32; 512];
        at_engine_rate.fill(&mut a);

        let mut explicit = AdaptiveMusic::new(48_000);
        explicit.add_layer(LoopBuffer::from_doc_at(&d, 48_000), 0.0);
        let mut b = vec![0.0f32; 512];
        explicit.fill(&mut b);
        assert_eq!(a, b, "add_layer_doc == from_doc_at at the engine rate");

        let mut wrong = AdaptiveMusic::new(48_000);
        wrong.add_layer(LoopBuffer::from_doc(&d), 0.0);
        let mut c = vec![0.0f32; 512];
        wrong.fill(&mut c);
        assert_ne!(a, c, "the doc-rate render is a different (detuned) signal");
    }

    #[test]
    fn transition_to_the_current_section_is_a_pure_noop() {
        // Requesting the section already playing must not start a fade: doing so
        // filled the same buffer twice per block and advanced its play head
        // twice (an audible speed-up). Output must equal a plain render.
        let plain = {
            let mut m = AdaptiveMusic::new(48_000);
            m.add_section("a", &tone(220.0));
            m.add_section("b", &tone(440.0));
            let mut o = vec![0.0f32; 256 * 2];
            m.fill(&mut o);
            o
        };
        let mut m = AdaptiveMusic::new(48_000);
        let a = m.add_section("a", &tone(220.0));
        m.add_section("b", &tone(440.0));
        m.transition_to(a, Quantize::Immediate); // `a` is already current
        let mut o = vec![0.0f32; 256 * 2];
        m.fill(&mut o);
        assert_eq!(
            o, plain,
            "a transition to the current section changed the mix"
        );
    }

    #[test]
    fn stinger_fires_on_the_beat() {
        let mut m = AdaptiveMusic::new(48_000);
        m.add_layer(LoopBuffer::from_doc(&tone(220.0)), 0.0);
        m.set_tempo(120.0, 4); // beat at 24 000 frames
        m.stinger_at(&tone(1320.0), Quantize::Beat);
        assert_eq!(m.active_stingers(), 0, "not yet — waiting for the beat");
        // Serve realistic blocks up to just before the beat.
        for _ in 0..46 {
            advance(&mut m, 512); // 23 552 frames < 24 000
        }
        assert_eq!(m.active_stingers(), 0, "still before the beat");
        advance(&mut m, 512); // crosses 24 000 — the stinger fires (and is long)
        assert_eq!(m.active_stingers(), 1, "stinger fired on the beat");
    }

    #[test]
    fn quantized_schedule_is_deterministic() {
        let run = || {
            let mut m = AdaptiveMusic::new(48_000);
            m.add_section("a", &tone(330.0));
            let b = m.add_section("b", &tone(660.0));
            m.add_layer(LoopBuffer::from_doc(&tone(220.0)), 0.0);
            m.set_tempo(140.0, 4);
            m.set_intensity_at(0.8, Quantize::Beat);
            m.transition_to(b, Quantize::Bar);
            m.stinger_at(&tone(990.0), Quantize::Bars(2));
            let mut acc = Vec::new();
            let mut out = vec![0.0f32; 333 * 2]; // odd block size stresses boundaries
            for _ in 0..300 {
                m.fill(&mut out);
                acc.extend_from_slice(&out);
            }
            acc
        };
        assert_eq!(
            run(),
            run(),
            "a fixed tempo + block schedule replays identically"
        );
    }

    #[test]
    fn immediate_api_unchanged_without_tempo() {
        // No tempo set → quantized calls act immediately (back-compat).
        let mut m = AdaptiveMusic::new(48_000);
        m.add_layer(LoopBuffer::from_doc(&tone(220.0)), 0.5);
        m.set_intensity_at(1.0, Quantize::Bar); // no tempo → immediate
        assert_eq!(m.intensity(), 1.0, "no tempo → applies immediately");
    }

    #[test]
    fn from_doc_len_forces_the_grid_and_loops_at_it() {
        // A 0.5 s tone forced to 1000 frames: it loops exactly at the grid, so
        // block N is sample-identical to block N+1.
        let grid = 1000;
        let mut buf = LoopBuffer::from_doc_len(&tone(220.0), grid);
        let mut first = vec![0.0f32; grid * 2];
        let mut second = vec![0.0f32; grid * 2];
        buf.fill(&mut first);
        buf.fill(&mut second);
        assert_eq!(first, second, "the buffer loops exactly at the grid length");
    }

    #[test]
    fn stem_set_shares_one_beat_grid() {
        let mut music = AdaptiveMusic::new(48_000);
        music.set_tempo(120.0, 4); // 48000*60/120 = 24000 frames/beat

        let base = tone(220.0);
        let hi = tone(880.0);
        let grid = music.add_stem_set(&[(&base, 0.0), (&hi, 0.5)], 4.0);

        // 4 beats × 24000 = 96000 frames — the provable shared length.
        assert_eq!(grid, 96_000, "grid = duration_beats × frames_per_beat");
        assert_eq!(music.layer_gain(0), Some(1.0), "base always on");
        assert_eq!(music.layer_gain(1), Some(0.0), "hi silent until intensity");
        // Both stems are forced to `grid` frames, so their loops are phase-locked
        // by construction (see from_doc_len's loop test).
    }

    #[test]
    fn stem_set_without_tempo_falls_back_to_the_first_stem() {
        let mut music = AdaptiveMusic::new(48_000);
        let base = tone(220.0);
        let grid = music.add_stem_set(&[(&base, 0.0)], 4.0);
        assert!(
            grid > 0,
            "no tempo → grid falls back to the first stem's natural length"
        );
    }

    #[test]
    fn scheduled_events_are_block_size_invariant() {
        // The determinism guarantee: quantized actions fire at their exact
        // frame, so the same schedule renders identical audio no matter what
        // block size the host serves (a 128-frame AudioWorklet vs a 512-frame
        // cpal callback used to render different audio).
        let run = |block: usize| {
            let mut m = AdaptiveMusic::new(48_000);
            m.add_section("a", &tone(330.0));
            let b = m.add_section("b", &tone(660.0));
            m.add_layer(LoopBuffer::from_doc(&tone(220.0)), 0.0);
            m.add_layer(LoopBuffer::from_doc(&tone(880.0)), 0.5);
            m.set_tempo(120.0, 4);
            m.set_intensity_at(1.0, Quantize::Beat);
            m.stinger_at(&tone(990.0), Quantize::Beat);
            m.transition_to(b, Quantize::Bar);
            let mut acc = Vec::new();
            let mut out = vec![0.0f32; block * 2];
            for _ in 0..(200_000 / block) {
                m.fill(&mut out);
                acc.extend_from_slice(&out);
            }
            acc
        };
        let a = run(128);
        let b = run(512);
        let n = a.len().min(b.len());
        assert_eq!(
            a[..n],
            b[..n],
            "output must not depend on the host block size"
        );
    }

    #[test]
    fn transition_reversal_mid_fade_cancels_back() {
        // During a fade A→B the effective section is B — asking for A again
        // cancels the fade (running it in reverse), not an "already current"
        // no-op that lets B take over anyway.
        let mut m = AdaptiveMusic::new(48_000);
        let a = m.add_section("a", &tone(330.0));
        let b = m.add_section("b", &tone(660.0));
        m.transition_to(b, Quantize::Immediate);
        advance(&mut m, 512); // partway into the fade
        m.transition_to(a, Quantize::Immediate);
        for _ in 0..40 {
            advance(&mut m, 512);
        }
        assert_eq!(m.current_section(), Some(a), "cancelled back to a");
    }

    #[test]
    fn re_transition_to_the_fade_target_is_a_noop() {
        // transition_to(B) mid-fade must not restart B's fade (rewinding its
        // buffer and audibly restarting): the original fade just completes.
        let mut m = AdaptiveMusic::new(48_000);
        m.add_section("a", &tone(330.0));
        let b = m.add_section("b", &tone(660.0));
        m.transition_to(b, Quantize::Immediate);
        advance(&mut m, 512); // 512 frames of fade progress
        m.transition_to(b, Quantize::Immediate);
        advance(&mut m, 2500); // +512 > the 2880-frame fade — done if unrestarted
        assert_eq!(
            m.current_section(),
            Some(b),
            "the fade completed on its original clock"
        );
    }

    #[test]
    fn audio_source_reset_restarts_the_bed() {
        // Through `&mut dyn AudioSource` reset used to be a silent no-op.
        let mut m = AdaptiveMusic::new(48_000);
        m.add_layer(LoopBuffer::from_doc(&tone(220.0)), 0.0);
        m.stinger(&tone(990.0));
        advance(&mut m, 512);
        assert!(m.position_frames() > 0);
        let src: &mut dyn AudioSource = &mut m;
        src.reset();
        assert_eq!(
            m.position_frames(),
            0,
            "reset through the trait restarts the clock"
        );
        assert_eq!(m.active_stingers(), 0, "and clears stingers");
    }

    #[test]
    fn third_section_switch_completes_the_fade_first() {
        // Mid-fade A→B, transition_to(C): the in-flight fade completes (B's
        // partial contribution is not hard-cut), then C fades in.
        let mut m = AdaptiveMusic::new(48_000);
        m.add_section("a", &tone(330.0));
        let b = m.add_section("b", &tone(660.0));
        let c = m.add_section("c", &tone(990.0));
        m.transition_to(b, Quantize::Immediate);
        advance(&mut m, 512); // 512 into the 2880-frame fade
        m.transition_to(c, Quantize::Immediate);
        advance(&mut m, 2880); // past the original fade's end (at 2880)
        assert_eq!(
            m.current_section(),
            Some(b),
            "the in-flight fade completed before the onward transition"
        );
        for _ in 0..20 {
            advance(&mut m, 512);
        }
        assert_eq!(m.current_section(), Some(c), "then c faded in");
    }
}
