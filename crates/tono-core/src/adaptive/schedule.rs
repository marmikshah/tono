//! The beat/bar clock and quantized scheduling: tempo, position, the
//! `pending` queue of deferred actions, and the exact-frame `fire_due` that
//! lets [`AudioSource::fill`](crate::runtime::AudioSource::fill) render
//! identically at any host block size.

use super::{Action, AdaptiveMusic, SoundDoc, Stinger};

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
    /// Apply on beat `n`, counted from the last reset like the `Bars` grid
    /// (beat 0 is the first frame). Lets a caller compute the beat first and
    /// do slow work, such as rendering a stinger, before scheduling it. A beat
    /// the clock has already reached applies at once.
    AtBeat(u64),
}

impl AdaptiveMusic {
    // ---- Musical time & quantized scheduling ----

    /// Set the tempo so transitions can align to beats and bars. Required for any
    /// [`Quantize`] other than [`Immediate`](Quantize::Immediate).
    pub fn set_tempo(&mut self, bpm: f32, beats_per_bar: u32) {
        self.bpm = bpm.max(0.0);
        // A new grid, same position: the clock keeps counting from where it is.
        let position = self.clock.position_frames();
        self.clock = Self::clock_at(self.sample_rate, self.bpm, beats_per_bar);
        self.clock.seek_frame(position);
    }

    /// Frames per beat at the current tempo, or `None` if no tempo is set.
    pub(super) fn frames_per_beat(&self) -> Option<f64> {
        (self.bpm > 0.0).then(|| self.sample_rate as f64 * 60.0 / self.bpm as f64)
    }

    /// The musical position in beats since the last [`reset`](AdaptiveMusic::reset)
    /// (0 until a tempo is set).
    pub fn beats(&self) -> f64 {
        if self.bpm > 0.0 {
            self.clock.position_beats()
        } else {
            0.0
        }
    }

    /// The musical position in bars since the last [`reset`](AdaptiveMusic::reset)
    /// (0 until a tempo is set).
    pub fn bars(&self) -> f64 {
        if self.bpm > 0.0 {
            self.clock.position_bars()
        } else {
            0.0
        }
    }

    /// Frames rendered while playing since construction or the last
    /// [`reset`](AdaptiveMusic::reset) — the musical clock. Beats derive from it exactly
    /// through the transport, so a game's beat-locked schedule stays
    /// phase-aligned with what is sounding. Holds while paused.
    pub fn position_frames(&self) -> u64 {
        self.clock.position_frames()
    }

    /// The frame at which quantization `q` next lands — the next boundary
    /// strictly ahead (on a boundary, "next bar" means the following one), or
    /// the named beat for `AtBeat`, which may already be behind. Returns the
    /// current frame for `Immediate` or when no tempo is set.
    fn fire_frame(&self, q: Quantize) -> u64 {
        let now = self.clock.position_frames();
        if self.bpm <= 0.0 {
            return now;
        }
        match q {
            Quantize::Immediate => now,
            Quantize::Beat => self.clock.next_beat(),
            Quantize::Bar => self.clock.next_bar(1),
            Quantize::Bars(n) => self.clock.next_bar(n),
            Quantize::AtBeat(n) => self.clock.frame_at_beat(n as f64),
        }
    }

    /// Apply `action` now if the `q` boundary is already here, else defer it —
    /// the one immediate-vs-scheduled decision every quantized entry point and
    /// [`fire_due`](Self::fire_due) share.
    pub(super) fn apply_or_schedule(&mut self, q: Quantize, action: Action) {
        let fire_at = self.fire_frame(q);
        if fire_at <= self.clock.position_frames() {
            self.apply(action);
        } else {
            self.pending.push(fire_at, action);
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
    /// A boundary relative to the clock ([`Quantize::Beat`] and the rest) is
    /// resolved after the render, so a long render can land a beat late; render
    /// with [`render_stinger`](Self::render_stinger) and schedule with
    /// [`play_stinger_at`](Self::play_stinger_at) and [`Quantize::AtBeat`] instead.
    pub fn stinger_at(&mut self, doc: &SoundDoc, q: Quantize) {
        let stinger = self.render_stinger(doc);
        self.play_stinger_at(&stinger, q);
    }

    /// Schedule a stinger from pre-rendered stereo — render off the audio thread
    /// and hand the buffers in, so a real-time caller never renders under a lock.
    pub fn stinger_stereo_at(&mut self, left: Vec<f32>, right: Vec<f32>, q: Quantize) {
        self.play_stinger_at(&Stinger::from_stereo(&left, &right), q);
    }

    /// Fire every scheduled action due at the current position, in
    /// chronological order. [`fill`](crate::runtime::AudioSource::fill) calls this at each
    /// exact boundary frame (rendering sub-spans around them), so the output
    /// never depends on the host's block size.
    pub(super) fn fire_due(&mut self) {
        // The timeline keeps `(frame, submission)` order, so actions sharing a
        // frame fire in the order they were scheduled.
        let now = self.clock.position_frames();
        while let Some(due) = self.pending.pop_due(now) {
            self.apply(due.action);
        }
    }
}
