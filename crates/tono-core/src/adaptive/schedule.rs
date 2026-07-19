//! The beat/bar clock and quantized scheduling: tempo, position, the
//! `pending` queue of deferred actions, and the exact-frame `fire_due` that
//! lets [`AudioSource::fill`](crate::runtime::AudioSource::fill) render
//! identically at any host block size.

use super::{Action, AdaptiveMusic, Scheduled, SoundDoc, doc_at, render_stereo_pair};

/// When a scheduled change takes effect, relative to the musical clock. Anything
/// but [`Immediate`](Quantize::Immediate) needs a tempo ([`set_tempo`](AdaptiveMusic::set_tempo));
/// without one it degrades to `Immediate`.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Quantize {
    /// Apply on the next `fill` block (no beat alignment; the default).
    #[default]
    Immediate,
    /// Apply on the next beat boundary.
    Beat,
    /// Apply on the next bar boundary.
    Bar,
    /// Apply on the next `n`-bar grid boundary (bars counted from the last
    /// reset, so `Bars(2)` at 0.5 bars in fires at bar 2 — the grid keeps
    /// repeated schedules deterministic). `0`/`1` = the next bar.
    Bars(u32),
}

impl AdaptiveMusic {
    // ---- Musical time & quantized scheduling ----

    /// Set the tempo so transitions can align to beats and bars. Required for any
    /// [`Quantize`] other than [`Immediate`](Quantize::Immediate).
    pub fn set_tempo(&mut self, bpm: f32, beats_per_bar: u32) {
        self.bpm = bpm.max(0.0);
        self.beats_per_bar = beats_per_bar.max(1);
    }

    /// Frames per beat at the current tempo, or `None` if no tempo is set.
    pub(super) fn frames_per_beat(&self) -> Option<f64> {
        (self.bpm > 0.0).then(|| self.sample_rate as f64 * 60.0 / self.bpm as f64)
    }

    /// The musical position in beats since the last [`reset`](AdaptiveMusic::reset).
    pub fn beats(&self) -> f64 {
        self.frames_per_beat()
            .map_or(0.0, |fpb| self.position as f64 / fpb)
    }

    /// The musical position in bars since the last [`reset`](AdaptiveMusic::reset).
    pub fn bars(&self) -> f64 {
        self.beats() / self.beats_per_bar as f64
    }

    /// Frames rendered while playing since construction or the last
    /// [`reset`](AdaptiveMusic::reset) — the musical clock. Beats derive from it exactly
    /// (`beats = position / (sample_rate * 60 / bpm)`), so a game's beat-locked
    /// schedule stays phase-aligned with what is sounding. Holds while paused.
    pub fn position_frames(&self) -> u64 {
        self.position
    }

    /// The frame at which quantization `q` next lands — the next boundary
    /// strictly ahead (on a boundary, "next bar" means the following one).
    /// Returns the current frame only for `Immediate` or when no tempo is set.
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

    /// Schedule an [`Action`] at the next `q` boundary (fired in [`fill`](crate::runtime::AudioSource::fill)).
    fn schedule(&mut self, q: Quantize, action: Action) {
        let fire_at = self.fire_frame(q);
        self.pending.push(Scheduled { fire_at, action });
    }

    /// Apply `action` now if the `q` boundary is already here, else defer it —
    /// the one immediate-vs-scheduled decision every quantized entry point and
    /// [`fire_due`](Self::fire_due) share.
    pub(super) fn apply_or_schedule(&mut self, q: Quantize, action: Action) {
        if self.fire_frame(q) <= self.position {
            self.apply(action);
        } else {
            self.schedule(q, action);
        }
    }

    /// Perform an [`Action`] now.
    fn apply(&mut self, action: Action) {
        match action {
            Action::SetIntensity(x) => self.set_intensity(x),
            Action::Stinger(st) => self.stingers.push(st),
            Action::Transition { to } => self.begin_transition(to),
        }
    }

    /// Set the intensity on a beat/bar boundary (see [`set_intensity`](AdaptiveMusic::set_intensity)).
    pub fn set_intensity_at(&mut self, x: f32, q: Quantize) {
        self.apply_or_schedule(q, Action::SetIntensity(x));
    }

    /// Fire a stinger on a beat/bar boundary. The stinger is **rendered now** (off
    /// the audio thread); only its playback is deferred to the boundary.
    pub fn stinger_at(&mut self, doc: &SoundDoc, q: Quantize) {
        let (left, right) = render_stereo_pair(&doc_at(doc, self.sample_rate));
        self.stinger_stereo_at(left, right, q);
    }

    /// Schedule a stinger from pre-rendered stereo — render off the audio thread
    /// and hand the buffers in, so a real-time caller never renders under a lock.
    pub fn stinger_stereo_at(&mut self, left: Vec<f32>, right: Vec<f32>, q: Quantize) {
        self.apply_or_schedule(q, Action::Stinger(super::Stinger::new(left, right)));
    }

    /// Fire every scheduled action due at the current position, in
    /// chronological order. [`fill`](crate::runtime::AudioSource::fill) calls this at each
    /// exact boundary frame (rendering sub-spans around them), so the output
    /// never depends on the host's block size.
    pub(super) fn fire_due(&mut self) {
        if self.pending.iter().all(|s| s.fire_at > self.position) {
            return;
        }
        let mut due: Vec<Scheduled> = Vec::new();
        let mut i = 0;
        while i < self.pending.len() {
            if self.pending[i].fire_at <= self.position {
                due.push(self.pending.swap_remove(i));
            } else {
                i += 1;
            }
        }
        due.sort_by_key(|s| s.fire_at);
        for s in due {
            self.apply(s.action);
        }
    }
}
