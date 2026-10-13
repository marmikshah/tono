//! Variation tools: derive new takes from an existing sound.
//!
//! `mutate` never invents content — it perturbs an existing graph,
//! deterministically from a seed, for round-robin sets and "nudge until it's
//! right" exploration.

use crate::dsl::{Modulator, Node, SoundDoc, Value};
use crate::dsp::Rng;

/// Salt the variation PRNG stream so it never collides with the renderer's
/// noise stream for the same seed.
fn salted_rng(seed: u64) -> Rng {
    Rng::new(seed ^ 0xA076_1D64_78BD_642F)
}

/// Return a perturbed copy of `doc`: numeric parameters are jittered by up to
/// `amount` (0..1) of their value, deterministically from `seed`. Ranges are
/// clamped so the result stays valid.
pub fn mutate(doc: &SoundDoc, amount: f32, seed: u64) -> SoundDoc {
    let amount = amount.clamp(0.0, 1.0);
    let mut rng = salted_rng(seed);
    let mut out = doc.clone();
    out.name = format!("{}_mut", doc.name);
    out.seed = doc.seed.wrapping_add(seed | 1);
    mutate_node(&mut out.root, amount, &mut rng);
    out
}

/// Scale `v` by a random factor within ±amount, clamped into `[min,
/// f32::MAX]`: an uncapped param near the f32 ceiling must overflow to the
/// ceiling, not to ±inf — `mutate` promises a still-valid document, and
/// validation requires finite.
fn jitter(v: f32, amount: f32, rng: &mut Rng, min: f32) -> f32 {
    (v * (1.0 + rng.bi() * amount)).clamp(min, f32::MAX)
}

fn jitter_unit(v: f32, amount: f32, rng: &mut Rng) -> f32 {
    (v + rng.bi() * amount).clamp(0.0, 1.0)
}

/// Mutate a duty-cycle `Value`, keeping a constant in a usable pulse range.
fn mutate_duty(duty: &mut Value, amount: f32, rng: &mut Rng) {
    match duty {
        Value::Const(c) => *c = jitter_unit(*c, amount, rng).clamp(0.05, 0.95),
        other => mutate_value(other, amount, rng),
    }
}

fn mutate_value(value: &mut Value, amount: f32, rng: &mut Rng) {
    match value {
        Value::Const(c) => *c = jitter(*c, amount, rng, 0.0),
        // Note names are musical intent — keep them stable under mutation.
        Value::Note(_) => {}
        Value::Modulated(Modulator::Slide { from, to, secs, .. }) => {
            *from = jitter(*from, amount, rng, 0.0);
            *to = jitter(*to, amount, rng, 0.0);
            *secs = jitter(*secs, amount, rng, 0.001);
        }
        Value::Modulated(Modulator::Lfo {
            rate,
            depth,
            center,
            ..
        }) => {
            *rate = jitter(*rate, amount, rng, 0.01);
            *depth = jitter(*depth, amount, rng, 0.0);
            *center = jitter(*center, amount, rng, 0.0);
        }
        Value::Modulated(Modulator::Arp { steps, rate }) => {
            for s in steps.iter_mut() {
                *s = jitter(*s, amount, rng, 0.0);
            }
            *rate = jitter(*rate, amount, rng, 0.01);
        }
        Value::Modulated(Modulator::EnvMod { from, to, .. }) => {
            *from = jitter(*from, amount, rng, 0.0);
            *to = jitter(*to, amount, rng, 0.0);
        }
        Value::Modulated(Modulator::Rand { from, to, rate, .. }) => {
            *from = jitter(*from, amount, rng, 0.0);
            *to = jitter(*to, amount, rng, 0.0);
            // Clamped to the validation cap — jitter runs to ×2.
            *rate = jitter(*rate, amount, rng, 0.01).min(10_000.0);
        }
    }
}

/// Mutate a frequency-bearing `Value`, then clamp it into the validation
/// domain (const (0, 100 kHz], modulated endpoints ±1 MHz): `mutate` promises
/// a still-valid document, and an unclamped jitter could push a near-cap
/// frequency past the bound (or an exact 0 into a positive-only field).
fn mutate_freq(value: &mut Value, amount: f32, rng: &mut Rng) {
    mutate_value(value, amount, rng);
    fn clamp_ep(x: &mut f32) {
        *x = (*x).clamp(-1e6, 1e6);
    }
    match value {
        Value::Const(c) => *c = (*c).clamp(1e-3, 100_000.0),
        Value::Modulated(Modulator::Slide { from, to, .. })
        | Value::Modulated(Modulator::EnvMod { from, to, .. })
        | Value::Modulated(Modulator::Rand { from, to, .. }) => {
            clamp_ep(from);
            clamp_ep(to);
        }
        Value::Modulated(Modulator::Lfo { center, depth, .. }) => {
            clamp_ep(center);
            clamp_ep(depth);
        }
        Value::Modulated(Modulator::Arp { steps, .. }) => steps.iter_mut().for_each(clamp_ep),
        Value::Note(_) => {}
    }
}

fn mutate_node(node: &mut Node, amount: f32, rng: &mut Rng) {
    match node {
        Node::Square { freq, duty } => {
            mutate_freq(freq, amount, rng);
            mutate_duty(duty, amount, rng);
        }
        Node::Triangle { freq } | Node::Sawtooth { freq } | Node::Sine { freq } => {
            mutate_freq(freq, amount, rng)
        }
        Node::Noise { .. } => {}
        Node::Fm { freq, index, .. } => {
            mutate_freq(freq, amount, rng);
            mutate_value(index, amount, rng);
        }
        Node::Seq {
            duty, env, notes, ..
        } => {
            mutate_duty(duty, amount, rng);
            env.a = jitter(env.a, amount, rng, 0.0);
            env.d = jitter(env.d, amount, rng, 0.0);
            env.r = jitter(env.r, amount, rng, 0.0);
            for note in notes.iter_mut() {
                mutate_value(&mut note.pitch, amount, rng);
            }
        }
        Node::Drive { amount: amt, .. } => mutate_value(amt, amount, rng),
        Node::Env { adsr } => {
            adsr.a = jitter(adsr.a, amount, rng, 0.0);
            adsr.d = jitter(adsr.d, amount, rng, 0.0);
            adsr.s = jitter_unit(adsr.s, amount, rng);
            adsr.r = jitter(adsr.r, amount, rng, 0.0);
            adsr.punch = jitter_unit(adsr.punch, amount, rng);
        }
        Node::Lowpass { cutoff, q }
        | Node::Highpass { cutoff, q }
        | Node::Bandpass { cutoff, q }
        | Node::Notch { cutoff, q } => {
            mutate_freq(cutoff, amount, rng);
            *q = jitter(*q, amount, rng, 0.05);
        }
        Node::Peak { cutoff, q, gain_db } => {
            mutate_freq(cutoff, amount, rng);
            *q = jitter(*q, amount, rng, 0.05);
            // Clamped to the ±24 dB validation bound: mutate promises a
            // still-valid document.
            *gain_db = (*gain_db + rng.bi() * amount * 6.0).clamp(-24.0, 24.0);
        }
        Node::Lowshelf { cutoff, gain_db } | Node::Highshelf { cutoff, gain_db } => {
            mutate_freq(cutoff, amount, rng);
            *gain_db = (*gain_db + rng.bi() * amount * 6.0).clamp(-24.0, 24.0);
        }
        Node::Super {
            freq, detune_cents, ..
        } => {
            mutate_freq(freq, amount, rng);
            // Clamped to the validation cap (10 octaves) — jitter runs to ×2.
            *detune_cents = jitter(*detune_cents, amount, rng, 0.0).min(12_000.0);
        }
        Node::Wavetable { freq, position, .. } => {
            mutate_freq(freq, amount, rng);
            // Jitter a constant morph position (unit-clamped); a modulated
            // position is musical motion — leave it alone, like note names.
            if let Value::Const(c) = position {
                *c = jitter_unit(*c, amount, rng);
            }
        }
        Node::Gain { amount: amt } => mutate_value(amt, amount, rng),
        Node::Bitcrush { .. } | Node::Downsample { .. } => {}
        Node::Delay { secs, feedback } => {
            // .min(30.0): the validation cap on the delay-line allocation.
            *secs = jitter(*secs, amount, rng, 0.001).min(30.0);
            *feedback = jitter_unit(*feedback, amount, rng);
        }
        Node::Reverb { room, mix } => {
            *room = jitter_unit(*room, amount, rng);
            *mix = jitter_unit(*mix, amount, rng);
        }
        Node::Modal { modes, mix } => {
            for m in modes.iter_mut() {
                // Positive floors: an exact 0 would fail validation.
                m.freq = jitter(m.freq, amount, rng, 1e-3);
                m.decay = jitter(m.decay, amount, rng, 1e-3);
                m.gain = jitter_unit(m.gain, amount, rng);
            }
            *mix = jitter_unit(*mix, amount, rng);
        }
        Node::Impact { hardness, velocity } => {
            *hardness = jitter_unit(*hardness, amount, rng);
            *velocity = jitter_unit(*velocity, amount, rng);
        }
        Node::Dust { density, decay } => {
            *density = jitter(*density, amount, rng, 1.0);
            *decay = jitter(*decay, amount, rng, 0.0);
        }
        Node::RingMod { freq } => mutate_freq(freq, amount, rng),
        Node::Tremolo { rate, depth } => {
            // .min(40.0): the validation cap on the LFO rate.
            *rate = jitter(*rate, amount, rng, 0.01).min(40.0);
            *depth = jitter_unit(*depth, amount, rng);
        }
        Node::Chorus { rate, depth, mix } => {
            *rate = jitter(*rate, amount, rng, 0.01);
            *depth = jitter_unit(*depth, amount, rng);
            *mix = jitter_unit(*mix, amount, rng);
        }
        Node::Flanger {
            rate,
            depth,
            feedback,
            mix,
        }
        | Node::Phaser {
            rate,
            depth,
            feedback,
            mix,
        } => {
            *rate = jitter(*rate, amount, rng, 0.01);
            *depth = jitter_unit(*depth, amount, rng);
            *feedback = jitter_unit(*feedback, amount, rng);
            *mix = jitter_unit(*mix, amount, rng);
        }
        Node::Compress { .. } => {}
        Node::Duck { amount: amt, .. } => {
            *amt = jitter_unit(*amt, amount, rng);
        }
        Node::Convolve {
            decay,
            size,
            predelay,
            damp,
            mix,
        } => {
            // .min(30.0): the validation caps on the IR allocation.
            *decay = jitter(*decay, amount, rng, 0.001).min(30.0);
            // 0 means "follow decay" — keep it non-negative.
            *size = jitter(*size, amount, rng, 0.0).min(30.0);
            *predelay = jitter(*predelay, amount, rng, 0.0).min(30.0);
            *damp = jitter_unit(*damp, amount, rng);
            *mix = jitter_unit(*mix, amount, rng);
        }
        Node::Granular {
            grain_ms,
            density,
            pitch,
            spread,
            mix,
        } => {
            *grain_ms = jitter(*grain_ms, amount, rng, 5.0).min(500.0);
            *density = jitter(*density, amount, rng, 0.1).min(200.0);
            *pitch = jitter(*pitch, amount, rng, 0.25).min(4.0);
            *spread = jitter_unit(*spread, amount, rng);
            *mix = jitter_unit(*mix, amount, rng);
        }
        // Combinators carry no knobs of their own; the shared traversal below
        // recurses into them. (Named explicitly — no `_`, so a new variant
        // still forces a mutation decision here.)
        Node::Mix { .. } | Node::Mul { .. } | Node::Chain { .. } | Node::Tracks { .. } => {}
    }
    // The recursion goes through the shared `children_mut` traversal (the
    // draw order above is unchanged: each variant's own knobs jitter first,
    // then children in document order).
    node.children_mut()
        .for_each(|c| mutate_node(c, amount, rng));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn zap() -> SoundDoc {
        serde_json::from_str(
            r#"{ "name": "zap", "duration": 0.2, "seed": 5, "root": { "type": "mul", "inputs": [
                { "type": "square",
                  "freq": { "slide": { "from": 880, "to": 180, "secs": 0.18 } }, "duty": 0.25 },
                { "type": "env", "d": 0.18, "punch": 0.3 }
            ] } }"#,
        )
        .unwrap()
    }

    #[test]
    fn mutate_is_deterministic_and_stays_valid() {
        let a = mutate(&zap(), 0.3, 7);
        let b = mutate(&zap(), 0.3, 7);
        assert_eq!(
            serde_json::to_value(&a).unwrap(),
            serde_json::to_value(&b).unwrap()
        );
        let c = mutate(&zap(), 0.3, 8);
        assert_ne!(
            serde_json::to_value(&a).unwrap(),
            serde_json::to_value(&c).unwrap()
        );
        assert_eq!(a.validate(), Ok(()));
        assert_eq!(a.name, "zap_mut");
        assert_ne!(a.seed, zap().seed); // derived noise seed differs too
    }

    #[test]
    fn mutate_perturbs_parameters_within_amount() {
        let m = mutate(&zap(), 0.2, 3);
        let v = serde_json::to_value(&m).unwrap();
        let from = v["root"]["inputs"][0]["freq"]["slide"]["from"]
            .as_f64()
            .unwrap() as f32;
        assert!(from != 880.0 && (from - 880.0).abs() <= 880.0 * 0.2 + 1.0);
    }

    #[test]
    fn mutate_stays_valid_with_a_delay_and_near_cap_knobs() {
        // The doc contract: "Ranges are clamped so the result stays valid" —
        // exercise the caps that used to break it (delay.secs × 2 over the
        // 30 s bound, detune/rand-rate over the overflow-regime caps).
        let d: SoundDoc = serde_json::from_str(
            r#"{ "name": "d", "duration": 0.3, "root": { "type": "chain", "stages": [
                { "type": "super", "freq": 110, "detune_cents": 9000 },
                { "type": "lowpass",
                  "cutoff": { "rand": { "from": 200, "to": 1200, "rate": 9000 } } },
                { "type": "delay", "secs": 25.0, "feedback": 0.4 }
            ] } }"#,
        )
        .unwrap();
        for seed in 0..50 {
            let m = mutate(&d, 1.0, seed);
            m.validate().unwrap_or_else(|e| panic!("seed {seed}: {e}"));
        }
    }

    #[test]
    fn mutate_stays_valid_with_a_tremolo() {
        // rate near the 40 Hz cap exercises the validation-bound clamp.
        let d: SoundDoc = serde_json::from_str(
            r#"{ "name": "t", "duration": 0.3, "root": { "type": "chain", "stages": [
                { "type": "sine", "freq": 220 },
                { "type": "tremolo", "rate": 38.0, "depth": 0.9 }
            ] } }"#,
        )
        .unwrap();
        for seed in 0..50 {
            let m = mutate(&d, 1.0, seed);
            m.validate().unwrap_or_else(|e| panic!("seed {seed}: {e}"));
        }
    }

    #[test]
    fn mutate_stays_valid_with_convolve_and_granular() {
        // Near-cap knobs exercise the validation-bound clamps (30 s IR times,
        // grain/density/pitch ranges).
        let d: SoundDoc = serde_json::from_str(
            r#"{ "name": "d", "duration": 0.3, "root": { "type": "chain", "stages": [
                { "type": "noise" },
                { "type": "convolve", "decay": 28.0, "size": 29.0, "predelay": 0.02,
                  "damp": 0.9, "mix": 0.4 },
                { "type": "granular", "grain_ms": 480.0, "density": 190.0, "pitch": 3.9,
                  "spread": 0.9, "mix": 0.6 }
            ] } }"#,
        )
        .unwrap();
        for seed in 0..50 {
            let m = mutate(&d, 1.0, seed);
            m.validate().unwrap_or_else(|e| panic!("seed {seed}: {e}"));
        }
    }
}
