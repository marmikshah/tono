//! Playable synthesis: a stateful, **gated** streaming voice — the foundation
//! for holding and playing notes like an instrument (Phase 1b).
//!
//! Unlike the offline renderer, which evaluates a fixed-duration document in one
//! whole-buffer pass, a [`Voice`] runs block-by-block and responds to gate
//! events in real time: `gate_on` starts the attack; the note sustains while
//! held; `gate_off` enters the release. The oscillator reuses the engine's exact
//! band-limited (PolyBLEP) kernels, so a held note's timbre matches a rendered
//! one. This module does not touch the offline render path — auditioning a fixed
//! document stays byte-identical to its bounce; this is the *live performance*
//! path, which a recorded gate sequence can later bounce deterministically.

use crate::dsl::{Adsr, Shape};
use crate::render::{osc, poly_blep};

/// A band-limited oscillator carrying its own phase — the per-sample kernels
/// from the renderer (`square_signal` / `saw_signal` / `tri_signal` / sine), in
/// stateful form so phase persists across blocks.
pub struct BandOsc {
    shape: Shape,
    phase: f32,
    tri: f32,
}

impl BandOsc {
    /// A fresh oscillator of `shape` at phase 0.
    pub fn new(shape: Shape) -> Self {
        Self {
            shape,
            phase: 0.0,
            tri: 0.0,
        }
    }

    /// Next sample at `freq` Hz (sample rate `sr`); `duty` applies to `Square`.
    pub fn tick(&mut self, freq: f32, sr: f32, duty: f32) -> f32 {
        let dt = freq.max(0.0) / sr;
        let value = match self.shape {
            Shape::Sine => osc(Shape::Sine, self.phase),
            Shape::Square => {
                let duty = duty.clamp(0.01, 0.99);
                let mut v = if self.phase < duty { 1.0 } else { -1.0 };
                v += poly_blep(self.phase, dt);
                v -= poly_blep((self.phase - duty + 1.0).fract(), dt);
                v
            }
            Shape::Saw => (2.0 * self.phase - 1.0) - poly_blep(self.phase, dt),
            Shape::Triangle => {
                let mut sq = if self.phase < 0.5 { 1.0 } else { -1.0 };
                sq += poly_blep(self.phase, dt);
                sq -= poly_blep((self.phase + 0.5).fract(), dt);
                self.tri = self.tri * 0.9995 + 4.0 * dt * sq;
                self.tri
            }
        };
        self.phase += dt;
        self.phase -= self.phase.floor();
        value
    }
}

/// Stage of a gated ADSR envelope.
#[derive(Clone, Copy, PartialEq, Debug)]
enum Stage {
    Idle,
    Attack,
    Decay,
    Sustain,
    Release,
}

/// A gate-driven ADSR envelope generator. Per-sample, linear segments — the
/// synth-voice counterpart to the document's fixed-duration `adsr`: here the
/// sustain holds for as long as the note is gated, and release is triggered by
/// `gate_off` rather than fixed at the buffer's end.
pub struct EnvGen {
    a: f32,
    d: f32,
    s: f32,
    r: f32,
    sr: f32,
    stage: Stage,
    level: f32,
    /// Level captured at `gate_off`, so release ramps from wherever it was.
    rel_from: f32,
}

impl EnvGen {
    /// An envelope from a document [`Adsr`] (the `punch` transient is ignored —
    /// that belongs to one-shot renders, not held notes).
    pub fn new(env: &Adsr, sr: u32) -> Self {
        Self {
            a: env.a.max(0.0),
            d: env.d.max(0.0),
            s: env.s.clamp(0.0, 1.0),
            r: env.r.max(0.0),
            sr: sr as f32,
            stage: Stage::Idle,
            level: 0.0,
            rel_from: 0.0,
        }
    }

    /// Strike the note: (re)start the attack from the current level.
    pub fn gate_on(&mut self) {
        self.stage = Stage::Attack;
    }

    /// Release the note: ramp down from the current level.
    pub fn gate_off(&mut self) {
        if self.stage != Stage::Idle {
            self.rel_from = self.level;
            self.stage = Stage::Release;
        }
    }

    /// Steal declick: cap the release at ~5 ms and gate off, so a stolen voice
    /// ramps out instead of being cut mid-sample (an audible click).
    pub fn kill(&mut self) {
        self.r = self.r.min(0.005);
        self.gate_off();
    }

    /// True until the release has fully decayed to silence.
    pub fn active(&self) -> bool {
        self.stage != Stage::Idle
    }

    /// The current envelope level in `[0, 1]` (for picking the quietest voice).
    pub fn level(&self) -> f32 {
        self.level
    }

    /// True once a held note has decayed to silence and will stay silent until
    /// released — a percussive envelope (`sustain ≈ 0`) that reached its sustain
    /// stage. Lets a polyphonic host reclaim one-shot voices that never get a
    /// matching note-off.
    pub fn faded(&self) -> bool {
        self.stage == Stage::Sustain && self.s <= 1e-4
    }

    /// Advance one sample and return the envelope level in `[0, 1]`.
    pub fn tick(&mut self) -> f32 {
        match self.stage {
            Stage::Idle => self.level = 0.0,
            Stage::Attack => {
                if self.a <= 0.0 {
                    self.level = 1.0;
                    self.stage = Stage::Decay;
                } else {
                    self.level += 1.0 / (self.a * self.sr);
                    if self.level >= 1.0 {
                        self.level = 1.0;
                        self.stage = Stage::Decay;
                    }
                }
            }
            Stage::Decay => {
                if self.d <= 0.0 {
                    self.level = self.s;
                    self.stage = Stage::Sustain;
                } else {
                    self.level -= (1.0 - self.s) / (self.d * self.sr);
                    if self.level <= self.s {
                        self.level = self.s;
                        self.stage = Stage::Sustain;
                    }
                }
            }
            Stage::Sustain => self.level = self.s,
            Stage::Release => {
                if self.r <= 0.0 {
                    self.level = 0.0;
                    self.stage = Stage::Idle;
                } else {
                    self.level -= self.rel_from / (self.r * self.sr);
                    if self.level <= 0.0 {
                        self.level = 0.0;
                        self.stage = Stage::Idle;
                    }
                }
            }
        }
        self.level
    }
}

/// A monophonic playable voice: one band-limited oscillator shaped by a gated
/// envelope. `process` *adds* into the output block, so a polyphonic engine can
/// sum many voices.
pub struct Voice {
    osc: BandOsc,
    env: EnvGen,
    sr: f32,
    freq: f32,
    duty: f32,
}

impl Voice {
    /// A voice with oscillator `shape` and envelope `env` at sample rate `sr`.
    pub fn new(shape: Shape, env: &Adsr, sr: u32) -> Self {
        Self {
            osc: BandOsc::new(shape),
            env: EnvGen::new(env, sr),
            sr: sr as f32,
            freq: 440.0,
            duty: 0.5,
        }
    }

    /// Strike a note at `freq` Hz.
    pub fn gate_on(&mut self, freq: f32) {
        self.freq = freq;
        self.env.gate_on();
    }

    /// Release the note (enters the envelope's release stage).
    pub fn gate_off(&mut self) {
        self.env.gate_off();
    }

    /// Pulse-width for a `Square` voice.
    pub fn set_duty(&mut self, duty: f32) {
        self.duty = duty;
    }

    /// True while the voice is still sounding (envelope not yet idle).
    pub fn active(&self) -> bool {
        self.env.active()
    }

    /// Add this voice's samples into a mono output block.
    pub fn process(&mut self, out: &mut [f32]) {
        for sample in out.iter_mut() {
            let e = self.env.tick();
            *sample += self.osc.tick(self.freq, self.sr, self.duty) * e;
        }
    }
}

/// A polyphonic synthesizer: a fixed pool of [`Voice`]s with note allocation
/// and voice-stealing. Notes are addressed by a caller-chosen `key` (e.g. a MIDI
/// note number or keyboard code) so `note_off` can release the right voice. This
/// is the engine a keyboard / MIDI input drives to play an instrument live.
pub struct PolySynth {
    voices: Vec<Voice>,
    /// The key each voice is currently assigned to (`None` once released/free).
    keys: Vec<Option<u32>>,
    /// Round-robin pointer for stealing when every voice is busy.
    steal: usize,
}

impl PolySynth {
    /// A synth of `max_voices` voices, each an oscillator `shape` + envelope.
    pub fn new(shape: Shape, env: &Adsr, sr: u32, max_voices: usize) -> Self {
        let n = max_voices.max(1);
        Self {
            voices: (0..n).map(|_| Voice::new(shape, env, sr)).collect(),
            keys: vec![None; n],
            steal: 0,
        }
    }

    /// Strike `key` at `freq` Hz — uses a free voice, or steals round-robin.
    pub fn note_on(&mut self, key: u32, freq: f32) {
        let slot = self
            .voices
            .iter()
            .position(|v| !v.active())
            .unwrap_or_else(|| {
                let s = self.steal;
                self.steal = (self.steal + 1) % self.voices.len();
                s
            });
        self.voices[slot].gate_on(freq);
        self.keys[slot] = Some(key);
    }

    /// Release the voice playing `key` (it enters its envelope's release).
    pub fn note_off(&mut self, key: u32) {
        for i in 0..self.voices.len() {
            if self.keys[i] == Some(key) {
                self.voices[i].gate_off();
                self.keys[i] = None;
            }
        }
    }

    /// Pulse-width for `Square` voices (applies to all).
    pub fn set_duty(&mut self, duty: f32) {
        for v in &mut self.voices {
            v.set_duty(duty);
        }
    }

    /// Number of voices still sounding (held or releasing).
    pub fn active_voices(&self) -> usize {
        self.voices.iter().filter(|v| v.active()).count()
    }

    /// Render the next mono block — sums every active voice (overwrites `out`).
    pub fn process(&mut self, out: &mut [f32]) {
        out.fill(0.0);
        for v in &mut self.voices {
            if v.active() {
                v.process(out);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env(a: f32, d: f32, s: f32, r: f32) -> Adsr {
        Adsr {
            a,
            d,
            s,
            r,
            punch: 0.0,
        }
    }

    #[test]
    fn gated_envelope_walks_attack_decay_sustain_release() {
        let sr = 48_000u32;
        let mut e = EnvGen::new(&env(0.01, 0.02, 0.5, 0.05), sr);
        assert!(!e.active(), "idle before gate");

        e.gate_on();
        // Run through attack: peak should reach ~1.0.
        let mut peak = 0.0f32;
        for _ in 0..(0.01 * sr as f32) as usize + 2 {
            peak = peak.max(e.tick());
        }
        assert!(peak > 0.98, "attack reaches unity, got {peak}");

        // Run past decay: should settle to sustain.
        for _ in 0..(0.02 * sr as f32) as usize + 4 {
            e.tick();
        }
        let sustain = e.tick();
        assert!((sustain - 0.5).abs() < 0.02, "sustain ≈ 0.5, got {sustain}");
        assert!(e.active(), "active while held");

        // Release to silence.
        e.gate_off();
        for _ in 0..(0.05 * sr as f32) as usize + 4 {
            e.tick();
        }
        assert_eq!(e.tick(), 0.0, "released to silence");
        assert!(!e.active(), "idle after release");
    }

    #[test]
    fn voice_sounds_while_held_and_falls_silent_after_release() {
        let sr = 48_000u32;
        let mut v = Voice::new(Shape::Sine, &env(0.005, 0.01, 0.6, 0.02), sr);
        v.gate_on(440.0);

        let mut block = vec![0.0f32; 1024];
        v.process(&mut block);
        let energy: f32 = block.iter().map(|x| x * x).sum();
        assert!(energy > 0.0, "held note produces sound");
        assert!(block.iter().all(|x| x.abs() <= 1.0), "bounded amplitude");

        // Release and run well past it — the voice goes idle, then a fresh
        // block is pure silence (an idle voice adds nothing).
        v.gate_off();
        let mut went_idle = false;
        for _ in 0..40 {
            let mut b = vec![0.0f32; 1024];
            v.process(&mut b);
            if !v.active() {
                went_idle = true;
                break;
            }
        }
        assert!(went_idle, "voice goes idle after gate_off");
        let mut after = vec![0.0f32; 1024];
        v.process(&mut after);
        assert!(after.iter().all(|x| *x == 0.0), "idle voice adds silence");
    }

    #[test]
    fn polysynth_plays_a_chord_and_steals_voices() {
        let sr = 48_000u32;
        let mut synth = PolySynth::new(Shape::Saw, &env(0.005, 0.02, 0.6, 0.03), sr, 3);

        // A three-note chord: all three voices sound.
        synth.note_on(60, 261.63);
        synth.note_on(64, 329.63);
        synth.note_on(67, 392.0);
        assert_eq!(synth.active_voices(), 3);
        let mut block = vec![0.0f32; 512];
        synth.process(&mut block);
        assert!(
            block.iter().map(|x| x * x).sum::<f32>() > 0.0,
            "chord sounds"
        );

        // A fourth note with only 3 voices steals one — never exceeds the pool.
        synth.note_on(72, 523.25);
        assert!(
            synth.active_voices() <= 3,
            "voice count bounded by the pool"
        );

        // Release everything → eventually silent and idle.
        for k in [60, 64, 67, 72] {
            synth.note_off(k);
        }
        let mut idle = false;
        for _ in 0..200 {
            let mut b = vec![0.0f32; 512];
            synth.process(&mut b);
            if synth.active_voices() == 0 {
                idle = true;
                break;
            }
        }
        assert!(idle, "all voices released to idle");
    }

    #[test]
    fn band_oscillator_is_bounded_and_periodic() {
        let mut o = BandOsc::new(Shape::Square);
        let sr = 48_000.0;
        let mut maxv = 0.0f32;
        for _ in 0..1000 {
            maxv = maxv.max(o.tick(220.0, sr, 0.5).abs());
        }
        assert!(maxv > 0.5 && maxv < 1.6, "square bounded, peak {maxv}");
    }
}
