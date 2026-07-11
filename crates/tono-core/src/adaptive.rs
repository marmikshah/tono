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

    /// Render `doc` and loop it at **exactly** `frames` samples — truncated or
    /// zero-padded to that length. Use it to force a set of stems onto one shared
    /// loop grid so their layered cross-fades stay sample-aligned (never drift
    /// phase). See [`AdaptiveMusic::add_stem_set`].
    pub fn from_doc_len(doc: &SoundDoc, frames: usize) -> Self {
        let p = render::render_product(doc);
        let (mut left, mut right) = p.stereo.unwrap_or_else(|| (p.mono.clone(), p.mono));
        left.resize(frames, 0.0);
        right.resize(frames, 0.0);
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

/// When a scheduled change takes effect, relative to the musical clock. Anything
/// but [`Immediate`](Quantize::Immediate) needs a tempo ([`set_tempo`](AdaptiveMusic::set_tempo));
/// without one it degrades to `Immediate`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Quantize {
    /// Apply on the next `fill` block (no beat alignment).
    Immediate,
    /// Apply on the next beat boundary.
    Beat,
    /// Apply on the next bar boundary.
    Bar,
    /// Apply on the boundary `n` bars from now (`0`/`1` = the next bar).
    Bars(u32),
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
    gain: f32,
    step: f32,
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

    /// Add a **phase-locked stem set**: every stem is rendered and forced onto
    /// one shared loop length, so their intensity cross-fades stay sample-aligned
    /// and never drift phase — the guarantee layered adaptive music needs.
    ///
    /// The shared length is `duration_beats` at the current tempo (set
    /// [`set_tempo`](Self::set_tempo) first); with no tempo it falls back to the
    /// first stem's natural rendered length. Each `(doc, fade_in_at)` becomes a
    /// layer, exactly as [`add_layer`](Self::add_layer). Returns the shared grid
    /// length in frames — schedule a beat clock against it.
    ///
    /// Because every stem is trimmed/padded to the same frame count, the set is
    /// **provably sample-identical in length** (author each `doc` to
    /// `duration_beats` so the fit is a trim, not a silent pad).
    pub fn add_stem_set(&mut self, stems: &[(&SoundDoc, f32)], duration_beats: f64) -> usize {
        let grid = self
            .frames_per_beat()
            .map(|fpb| (duration_beats * fpb).round() as usize)
            .filter(|f| *f > 0)
            .or_else(|| {
                stems
                    .first()
                    .map(|(doc, _)| render::render_product(doc).mono.len())
            })
            .unwrap_or(0);
        for (doc, fade_in_at) in stems {
            self.add_layer(LoopBuffer::from_doc_len(doc, grid), *fade_in_at);
        }
        grid
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
        // Attack toward the floor rather than stepping to it — an instantaneous
        // gain jump is an audible click. `fill` ramps `duck_gain` down over a few
        // ms, then releases back to unity.
        self.duck_target = (1.0 - depth).clamp(0.0, 1.0);
        let secs = release.as_secs_f32().max(1.0 / self.sample_rate as f32);
        self.duck_coeff = 1.0 - (-1.0 / (secs * self.sample_rate as f32)).exp();
        let attack = 0.002 * self.sample_rate as f32;
        self.duck_attack = 1.0 - (-1.0 / attack).exp();
    }

    // ---- Musical time & quantized scheduling ----

    /// Set the tempo so transitions can align to beats and bars. Required for any
    /// [`Quantize`] other than [`Immediate`](Quantize::Immediate).
    pub fn set_tempo(&mut self, bpm: f32, beats_per_bar: u32) {
        self.bpm = bpm.max(0.0);
        self.beats_per_bar = beats_per_bar.max(1);
    }

    /// Frames per beat at the current tempo, or `None` if no tempo is set.
    fn frames_per_beat(&self) -> Option<f64> {
        (self.bpm > 0.0).then(|| self.sample_rate as f64 * 60.0 / self.bpm as f64)
    }

    /// The musical position in beats since the last [`reset`](Self::reset).
    pub fn beats(&self) -> f64 {
        self.frames_per_beat()
            .map_or(0.0, |fpb| self.position as f64 / fpb)
    }

    /// The musical position in bars since the last [`reset`](Self::reset).
    pub fn bars(&self) -> f64 {
        self.beats() / self.beats_per_bar as f64
    }

    /// The frame at which quantization `q` next lands. Returns the current frame
    /// when already on the boundary or when no tempo is set (→ immediate).
    fn fire_frame(&self, q: Quantize) -> u64 {
        let period = match q {
            Quantize::Immediate => return self.position,
            Quantize::Beat => self.frames_per_beat(),
            Quantize::Bar => self
                .frames_per_beat()
                .map(|f| f * self.beats_per_bar as f64),
            Quantize::Bars(n) => self
                .frames_per_beat()
                .map(|f| f * self.beats_per_bar as f64 * n.max(1) as f64),
        };
        let Some(period) = period.filter(|p| *p >= 1.0) else {
            return self.position;
        };
        // The next boundary strictly ahead: on a boundary, "next bar" means the
        // following one (a full period away), not right now.
        let pos = self.position as f64;
        let boundary = (pos / period).floor() * period + period;
        boundary.round() as u64
    }

    /// Schedule an [`Action`] at the next `q` boundary (fired in [`fill`]).
    fn schedule(&mut self, q: Quantize, action: Action) {
        let fire_at = self.fire_frame(q);
        self.pending.push(Scheduled { fire_at, action });
    }

    /// Set the intensity on a beat/bar boundary (see [`set_intensity`](Self::set_intensity)).
    pub fn set_intensity_at(&mut self, x: f32, q: Quantize) {
        if self.fire_frame(q) <= self.position {
            self.set_intensity(x);
        } else {
            self.schedule(q, Action::SetIntensity(x));
        }
    }

    /// Fire a stinger on a beat/bar boundary. The stinger is **rendered now** (off
    /// the audio thread); only its playback is deferred to the boundary.
    pub fn stinger_at(&mut self, doc: &SoundDoc, q: Quantize) {
        if self.fire_frame(q) <= self.position {
            self.stinger(doc);
            return;
        }
        let p = render::render_product(doc);
        let (left, right) = p.stereo.unwrap_or_else(|| (p.mono.clone(), p.mono));
        self.schedule(
            q,
            Action::Stinger(Stinger {
                left,
                right,
                pos: 0,
            }),
        );
    }

    // ---- Horizontal sections ----

    /// Add a horizontal section (a looping bed). The first section added starts
    /// playing immediately; switch between them with [`transition_to`](Self::transition_to).
    pub fn add_section(&mut self, name: impl Into<String>, doc: &SoundDoc) -> usize {
        let index = self.sections.len();
        self.sections.push(Section {
            name: name.into(),
            buffer: LoopBuffer::from_doc(doc),
        });
        if self.current_section.is_none() {
            self.current_section = Some(index);
        }
        index
    }

    /// Cross-fade to another section on a beat/bar boundary — horizontal
    /// re-sequencing (e.g. swap "explore" for "battle" on the next bar). The target
    /// enters from its downbeat. A no-op for an unknown or already-current section.
    pub fn transition_to(&mut self, section: usize, q: Quantize) {
        if section >= self.sections.len() {
            return;
        }
        // A new transition supersedes any still-pending one (no stacking of
        // duplicate/contradictory transitions). Requesting the current section
        // just cancels a pending transition.
        self.pending
            .retain(|s| !matches!(s.action, Action::Transition { .. }));
        if self.current_section == Some(section) {
            return;
        }
        if self.fire_frame(q) <= self.position {
            self.begin_transition(section);
        } else {
            self.schedule(q, Action::Transition { to: section });
        }
    }

    /// Start the section cross-fade now: rewind the target to its head so it enters
    /// on its downbeat, and ramp it in over the declick window.
    fn begin_transition(&mut self, to: usize) {
        // Already there (a duplicate/queued transition): do nothing. Starting a
        // fade to the current section would fill the same buffer twice per block
        // and advance its play head twice — an audible speed-up.
        if self.current_section == Some(to) {
            return;
        }
        if self.current_section.is_none() {
            self.current_section = Some(to);
            return;
        }
        self.sections[to].buffer.reset();
        self.section_fade = Some(SectionFade {
            to,
            gain: 0.0,
            step: self.section_step,
        });
    }

    /// The section currently sounding, if any.
    pub fn current_section(&self) -> Option<usize> {
        self.current_section
    }

    /// Look up a section index by name.
    pub fn section_named(&self, name: &str) -> Option<usize> {
        self.sections.iter().position(|s| s.name == name)
    }

    /// Fire every scheduled action due within `[position, position + frames)`,
    /// in chronological order. Called at the top of each block.
    fn fire_due(&mut self, frames: usize) {
        let horizon = self.position + frames as u64;
        if self.pending.iter().all(|s| s.fire_at >= horizon) {
            return;
        }
        let mut due: Vec<Scheduled> = Vec::new();
        let mut i = 0;
        while i < self.pending.len() {
            if self.pending[i].fire_at < horizon {
                due.push(self.pending.swap_remove(i));
            } else {
                i += 1;
            }
        }
        due.sort_by_key(|s| s.fire_at);
        for s in due {
            match s.action {
                Action::SetIntensity(x) => self.set_intensity(x),
                Action::Stinger(st) => self.stingers.push(st),
                Action::Transition { to } => self.begin_transition(to),
            }
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
        // Apply any changes scheduled to land in this block before rendering it.
        self.fire_due(frames);
        if self.scratch.len() < frames * 2 {
            self.scratch.resize(frames * 2, 0.0);
        }
        let coeff = self.fade_coeff;
        let scratch = &mut self.scratch[..frames * 2];

        // Horizontal section (the base bed), optionally cross-fading to a new one.
        let fade = self.section_fade.as_ref().map(|f| (f.to, f.gain, f.step));
        if let Some((to, start, step)) = fade {
            if let Some(cur) = self.current_section {
                self.sections[cur].buffer.fill(scratch);
                for f in 0..frames {
                    let g = (start + f as f32 * step).min(1.0);
                    out[f * 2] += scratch[f * 2] * (1.0 - g);
                    out[f * 2 + 1] += scratch[f * 2 + 1] * (1.0 - g);
                }
            }
            self.sections[to].buffer.fill(scratch);
            for f in 0..frames {
                let g = (start + f as f32 * step).min(1.0);
                out[f * 2] += scratch[f * 2] * g;
                out[f * 2 + 1] += scratch[f * 2 + 1] * g;
            }
            let end = (start + frames as f32 * step).min(1.0);
            if end >= 1.0 {
                self.current_section = Some(to);
                self.section_fade = None;
            } else if let Some(fd) = self.section_fade.as_mut() {
                fd.gain = end;
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
        // Master duck: attack toward the floor, then release to unity. Skip
        // entirely when idle at unity so the common case pays nothing.
        if self.duck_gain < 1.0 || self.duck_target < 1.0 {
            for f in 0..frames {
                if self.duck_gain > self.duck_target {
                    // Attack: ramp down to the floor over a few ms (no click).
                    self.duck_gain += (self.duck_target - self.duck_gain) * self.duck_attack;
                    if self.duck_gain <= self.duck_target + 1e-4 {
                        self.duck_gain = self.duck_target;
                        // Floor reached — release from here on.
                        self.duck_target = 1.0;
                    }
                } else {
                    // Release: recover toward unity, snapping so it doesn't
                    // asymptote below 1.0 forever (a permanent tiny attenuation).
                    self.duck_gain += (1.0 - self.duck_gain) * self.duck_coeff;
                    if self.duck_gain >= 1.0 - 1e-4 {
                        self.duck_gain = 1.0;
                    }
                }
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
}
