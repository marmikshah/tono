//! Variation tools: derive new takes from an existing sound.
//!
//! These never invent content — they perturb (`mutate`) or coherently shift
//! (`humanize`) an existing graph, deterministically from a seed. Together with
//! `edit::morph` they power round-robin sets and "nudge until it's right"
//! exploration.

use crate::dsl::{Modulator, Node, SoundDoc, Value};
use crate::dsp::Rng;

/// Salt the variation PRNG stream so it never collides with the renderer's
/// noise stream for the same seed.
fn salted_rng(seed: u64) -> Rng {
    Rng::new(seed ^ 0xA076_1D64_78BD_642F)
}

/// A round-robin take. Rather than jittering every parameter independently
/// (which drifts off-character), humanize applies ONE coherent pitch shift
/// (± `pitch_cents`) to every pitched source and ONE level trim (± `gain_db`)
/// to the whole sound — exactly the variation a real performer produces
/// between repeats. Identity parameters (envelopes, filters, structure) are
/// untouched.
pub fn humanize(doc: &SoundDoc, pitch_cents: f32, gain_db: f32, seed: u64) -> SoundDoc {
    let mut rng = salted_rng(seed);
    let ratio = 2f32.powf((rng.bi() * pitch_cents) / 1200.0);
    let level = 10f32.powf((rng.bi() * gain_db) / 20.0);
    let mut out = doc.clone();
    out.name = format!("{}_h", doc.name);
    out.seed = doc.seed.wrapping_add(seed | 1);
    transpose_node(&mut out.root, ratio);
    let trim = Node::Gain {
        amount: Value::Const(level),
    };
    if let Node::Tracks { master, .. } = &mut out.root {
        // `tracks` must stay the root, so the level trim joins the master
        // chain (Gain is an ordinary processor) instead of wrapping the doc.
        master.push(trim);
    } else {
        let old = std::mem::replace(
            &mut out.root,
            Node::Mix { inputs: Vec::new() }, // placeholder, replaced below
        );
        out.root = Node::Chain {
            stages: vec![old, trim],
        };
    }
    out
}

/// Scale every pitch-bearing `Value` by `ratio` (note names resolve to Hz).
fn transpose_value(v: &mut Value, ratio: f32) {
    match v {
        Value::Const(c) => *c *= ratio,
        Value::Note(s) => {
            if let Some(hz) = crate::dsl::note_to_hz(s) {
                *v = Value::Const(hz * ratio);
            }
        }
        Value::Modulated(m) => match m {
            Modulator::Slide { from, to, .. } => {
                *from *= ratio;
                *to *= ratio;
            }
            Modulator::Lfo { center, depth, .. } => {
                *center *= ratio;
                *depth *= ratio;
            }
            Modulator::Arp { steps, .. } => steps.iter_mut().for_each(|s| *s *= ratio),
            Modulator::EnvMod { from, to, .. } => {
                *from *= ratio;
                *to *= ratio;
            }
            Modulator::Rand { from, to, .. } => {
                *from *= ratio;
                *to *= ratio;
            }
        },
    }
}

/// Recursively transpose only the pitched sources (oscillator frequencies and
/// seq note pitches) — filters / envelopes / levels are identity, not pitch.
fn transpose_node(node: &mut Node, ratio: f32) {
    match node {
        Node::Square { freq, .. }
        | Node::Triangle { freq }
        | Node::Sawtooth { freq }
        | Node::Sine { freq }
        | Node::Fm { freq, .. }
        | Node::Super { freq, .. } => transpose_value(freq, ratio),
        Node::Seq { notes, .. } => notes
            .iter_mut()
            .for_each(|n| transpose_value(&mut n.pitch, ratio)),
        Node::Mix { inputs } | Node::Mul { inputs } => {
            inputs.iter_mut().for_each(|n| transpose_node(n, ratio))
        }
        Node::Chain { stages } => stages.iter_mut().for_each(|n| transpose_node(n, ratio)),
        Node::Tracks { tracks, .. } => tracks
            .iter_mut()
            .for_each(|t| transpose_node(&mut t.node, ratio)),
        _ => {}
    }
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

/// Scale `v` by a random factor within ±amount, never below `min`.
fn jitter(v: f32, amount: f32, rng: &mut Rng, min: f32) -> f32 {
    (v * (1.0 + rng.bi() * amount)).max(min)
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
            *rate = jitter(*rate, amount, rng, 0.01);
        }
    }
}

fn mutate_node(node: &mut Node, amount: f32, rng: &mut Rng) {
    match node {
        Node::Square { freq, duty } => {
            mutate_value(freq, amount, rng);
            mutate_duty(duty, amount, rng);
        }
        Node::Triangle { freq } | Node::Sawtooth { freq } | Node::Sine { freq } => {
            mutate_value(freq, amount, rng)
        }
        Node::Noise { .. } => {}
        Node::Fm { freq, index, .. } => {
            mutate_value(freq, amount, rng);
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
        Node::Mix { inputs } | Node::Mul { inputs } => {
            inputs.iter_mut().for_each(|n| mutate_node(n, amount, rng))
        }
        Node::Chain { stages } => stages.iter_mut().for_each(|n| mutate_node(n, amount, rng)),
        Node::Lowpass { cutoff, q }
        | Node::Highpass { cutoff, q }
        | Node::Bandpass { cutoff, q }
        | Node::Notch { cutoff, q } => {
            mutate_value(cutoff, amount, rng);
            *q = jitter(*q, amount, rng, 0.05);
        }
        Node::Peak { cutoff, q, gain_db } => {
            mutate_value(cutoff, amount, rng);
            *q = jitter(*q, amount, rng, 0.05);
            // Clamped to the ±24 dB validation bound: mutate promises a
            // still-valid document.
            *gain_db = (*gain_db + rng.bi() * amount * 6.0).clamp(-24.0, 24.0);
        }
        Node::Lowshelf { cutoff, gain_db } | Node::Highshelf { cutoff, gain_db } => {
            mutate_value(cutoff, amount, rng);
            *gain_db = (*gain_db + rng.bi() * amount * 6.0).clamp(-24.0, 24.0);
        }
        Node::Super {
            freq, detune_cents, ..
        } => {
            mutate_value(freq, amount, rng);
            *detune_cents = jitter(*detune_cents, amount, rng, 0.0);
        }
        Node::Gain { amount: amt } => mutate_value(amt, amount, rng),
        Node::Bitcrush { .. } | Node::Downsample { .. } => {}
        Node::Delay { secs, feedback } => {
            *secs = jitter(*secs, amount, rng, 0.001);
            *feedback = jitter_unit(*feedback, amount, rng);
        }
        Node::Reverb { room, mix } => {
            *room = jitter_unit(*room, amount, rng);
            *mix = jitter_unit(*mix, amount, rng);
        }
        Node::Modal { modes, mix } => {
            for m in modes.iter_mut() {
                m.freq = jitter(m.freq, amount, rng, 0.0);
                m.decay = jitter(m.decay, amount, rng, 0.0);
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
        Node::RingMod { freq } => mutate_value(freq, amount, rng),
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
        Node::Tracks { tracks, master } => {
            for t in tracks.iter_mut() {
                mutate_node(&mut t.node, amount, rng);
            }
            for m in master.iter_mut() {
                mutate_node(m, amount, rng);
            }
        }
        Node::Duck {
            trigger,
            amount: amt,
            ..
        } => {
            *amt = jitter_unit(*amt, amount, rng);
            mutate_node(trigger, amount, rng);
        }
    }
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
    fn humanize_keeps_tracks_at_the_root() {
        let d: SoundDoc = serde_json::from_str(
            r#"{ "name": "band", "duration": 0.2, "root": { "type": "tracks",
                "tracks": [ { "node": { "type": "sine", "freq": 220 } } ],
                "master": [ { "type": "lowpass", "cutoff": 8000 } ] } }"#,
        )
        .unwrap();
        let h = humanize(&d, 30.0, 1.0, 9);
        // The level trim joins the master chain — wrapping the root would make
        // the doc invalid (`tracks` must stay the root node).
        assert_eq!(h.validate(), Ok(()));
        let Node::Tracks { master, .. } = &h.root else {
            panic!("tracks must stay the root");
        };
        assert_eq!(master.len(), 2);
        assert!(matches!(master.last(), Some(Node::Gain { .. })));
    }

    #[test]
    fn humanize_shifts_pitch_coherently_and_trims_level() {
        let h = humanize(&zap(), 50.0, 1.5, 11);
        assert_eq!(h.name, "zap_h");
        // Root is wrapped in a chain ending in one gain stage.
        let Node::Chain { stages } = &h.root else {
            panic!("expected chain wrapper");
        };
        assert!(matches!(stages[1], Node::Gain { .. }));
        // Both slide endpoints moved by the SAME ratio (coherent transpose).
        let v = serde_json::to_value(&h).unwrap();
        let slide = &v["root"]["stages"][0]["inputs"][0]["freq"]["slide"];
        let from = slide["from"].as_f64().unwrap() / 880.0;
        let to = slide["to"].as_f64().unwrap() / 180.0;
        assert!((from - to).abs() < 1e-4, "ratios {from} vs {to}");
        assert!((0.97..=1.03).contains(&from)); // within ±50 cents
    }
}
