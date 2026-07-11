//! Gated envelope generation — the per-voice building block of the live
//! instrument layer.
//!
//! Unlike the offline renderer's fixed-duration `adsr` (which anchors its
//! release to the end of the buffer), [`EnvGen`] is **gate-driven**: `gate_on`
//! starts the attack, the sustain holds for as long as the note is held, and
//! `gate_off` triggers the release. [`crate::instrument`] sums one of these
//! per voice. This module never touches the offline render path — auditioning
//! a fixed document stays byte-identical to its bounce.

use crate::dsl::Adsr;

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
    fn kill_shortens_a_long_release_to_a_declick_ramp() {
        let sr = 48_000u32;
        let mut e = EnvGen::new(&env(0.0, 0.0, 1.0, 2.0), sr); // 2 s release
        e.gate_on();
        e.tick();
        e.kill();
        // Drains within the ~5 ms window instead of 2 s.
        for _ in 0..(0.005 * sr as f32) as usize + 4 {
            e.tick();
        }
        assert!(!e.active(), "killed voice drains within the declick window");
    }
}
