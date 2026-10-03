//! Property-based fuzzing of the validation contract:
//!
//!   1. `serde_json::from_value::<SoundDoc>` + `validate()` never panic, on
//!      ANY JSON value.
//!   2. A document that PASSES validation renders finite samples without
//!      hanging.
//!   3. A document that FAILS validation (or was never validated) still can't
//!      panic the renderer — the render may be garbage, but the output stage
//!      (`peak_limit`, and the normalize stages that end in it) scrubs
//!      non-finite samples to silence, so the contract asserted here is:
//!      completes, and every output sample is finite.
//!   4. `vary::mutate` on a valid document always yields a document that
//!      still validates (its documented promise).
//!
//! Documents are generated as JSON (the wire format every consumer parses),
//! with f32 parameters drawn from pools that mix valid-ish values with the
//! nasty edge cases: ±0, denormals, 1e±38, huge, and out-of-range values.
//! serde_json can't represent NaN/±inf, so those enter through a poison pass
//! that mutates every reachable f32 field of a parsed document. Durations
//! stay tiny and sample rates in a sane band, so even the "valid" renders are
//! fast; the case budget is 64, matching the repo's frugal test times.
//!
//! Two genuine contract violations were found by this suite and fixed: a
//! single-point tracks automation lane with a NaN time panicked `lane_for`
//! (render/tracks.rs), and `vary::mutate`'s jitter overflowed uncapped-above
//! parameters to inf, breaking its still-valid promise. Both fixes are pinned
//! by the regression tests at the bottom of this file
//! (`single_point_nan_lane_renders_without_panic`,
//! `mutate_clamps_uncapped_params_to_finite`), and the properties fuzz the
//! formerly-excluded input classes along with everything else.

use proptest::prelude::*;
use serde_json::{Value as J, json};
use tono_core::dsl::{Adsr, Modulator, Node, Playback, SoundDoc, Stereo, Value};
use tono_core::{render, vary};

/// Finite edge-case numbers (serde_json can't hold NaN/±inf — those enter
/// through the poison pass instead).
const EDGE: &[f64] = &[
    0.0,
    -0.0,
    1.0,
    -1.0,
    0.5,
    -0.5,
    2.0,
    440.0,
    1e-38,
    -1e-38,
    1e-30,
    1e30,
    1e38,
    -1e38,
    3.4e38,
    f64::MIN_POSITIVE,
    30.0,
    30.0001,
    100_000.0,
    100_000.1,
    1e6,
];

/// Non-finite / extreme values injected into a parsed document's f32 fields.
const POISON: &[f32] = &[
    f32::NAN,
    f32::INFINITY,
    f32::NEG_INFINITY,
    f32::MAX,
    f32::MIN,
    1e-38,
    0.0,
    -0.0,
    -1.0,
];

const NOTES_OK: &[&str] = &[
    "A4", "C3", "C#3", "Gb5", "F#2", "E1", "midi:36", "midi:69", "m127",
];
const NOTES_ANY: &[&str] = &[
    "A4",
    "midi:69",
    "H9",
    "",
    "midi:-3",
    "midi:200",
    "A200000000",
    "C#",
    "m1e9",
];

const WAVES_OK: &[&str] = &[
    "square", "triangle", "sawtooth", "sine", "noise", "fm", "pluck", "piano", "epiano", "organ",
    "strings", "brass", "flute", "mallet", "bell", "bass", "cowbell", "kit",
];
const WAVES_ANY: &[&str] = &["square", "sine", "piano", "kit", "bass", "sampler", "bogus"];

type SJ = BoxedStrategy<J>;

fn config() -> ProptestConfig {
    ProptestConfig {
        cases: 64,
        // Keep the repo tree clean: failures print the seed instead of
        // writing a proptest-regressions file.
        failure_persistence: None,
        ..ProptestConfig::default()
    }
}

/// A number mostly inside `[lo, hi]` (the field's validation domain) when
/// `valid`; heavily spiced with edge cases when not.
fn fnum(valid: bool, lo: f64, hi: f64) -> BoxedStrategy<f64> {
    if valid {
        prop_oneof![
            12 => lo..=hi,
            1 => proptest::sample::select(EDGE.to_vec()),
        ]
        .boxed()
    } else {
        prop_oneof![
            3 => lo..=hi,
            2 => -2.0f64..2.0,
            2 => 0.0f64..20_000.0,
            3 => proptest::sample::select(EDGE.to_vec()),
        ]
        .boxed()
    }
}

/// A u32 inside the field's valid range when `valid`, sometimes outside it.
fn uint(valid: bool, vlo: u32, vhi: u32, alo: u32, ahi: u32) -> BoxedStrategy<u32> {
    if valid {
        (vlo..=vhi).boxed()
    } else {
        prop_oneof![2 => vlo..=vhi, 1 => alo..=ahi].boxed()
    }
}

fn with_fields(base: &J, extra: &[(&str, J)]) -> J {
    let mut o = base.as_object().cloned().unwrap_or_default();
    for (k, v) in extra {
        o.insert((*k).into(), v.clone());
    }
    J::Object(o)
}

fn adsr_json(valid: bool) -> SJ {
    (
        fnum(valid, 0.0, 0.5),
        fnum(valid, 0.0, 0.5),
        fnum(valid, 0.0, 1.0),
        fnum(valid, 0.0, 0.5),
        fnum(valid, 0.0, 1.0),
    )
        .prop_map(|(a, d, s, r, punch)| json!({ "a": a, "d": d, "s": s, "r": r, "punch": punch }))
        .boxed()
}

fn modulator_json(valid: bool) -> SJ {
    let n_steps = if valid { 1..=4usize } else { 0..=4usize };
    prop_oneof![
        (
            fnum(valid, -1e3, 1e4),
            fnum(valid, -1e3, 1e4),
            fnum(valid, 1e-3, 2.0),
            proptest::sample::select(vec!["lin", "exp"]),
        )
            .prop_map(|(from, to, secs, curve)| {
                json!({ "slide": { "from": from, "to": to, "secs": secs, "curve": curve } })
            })
            .boxed(),
        (
            proptest::sample::select(vec!["sine", "square", "triangle", "saw"]),
            fnum(valid, 0.01, 40.0),
            fnum(valid, -1e3, 1e3),
            fnum(valid, -1e3, 1e3),
        )
            .prop_map(|(shape, rate, depth, center)| {
                json!({ "lfo": { "shape": shape, "rate": rate, "depth": depth, "center": center } })
            })
            .boxed(),
        (
            proptest::collection::vec(fnum(valid, -1e3, 1e4), n_steps),
            fnum(valid, 0.01, 100.0),
        )
            .prop_map(|(steps, rate)| json!({ "arp": { "steps": steps, "rate": rate } }))
            .boxed(),
        (
            adsr_json(valid),
            fnum(valid, -1e3, 1e3),
            fnum(valid, -1e3, 1e3),
        )
            .prop_map(|(e, from, to)| {
                json!({ "env": with_fields(&e, &[("from", json!(from)), ("to", json!(to))]) })
            })
            .boxed(),
        (
            fnum(valid, -1e3, 1e3),
            fnum(valid, -1e3, 1e3),
            fnum(valid, 0.01, 9_000.0),
            any::<u64>(),
        )
            .prop_map(|(from, to, rate, seed)| {
                json!({ "rand": { "from": from, "to": to, "rate": rate, "seed": seed } })
            })
            .boxed(),
    ]
    .boxed()
}

/// A dsl `Value` in JSON: constant, note name, or modulator.
fn value_json(valid: bool, lo: f64, hi: f64) -> SJ {
    let notes = if valid { NOTES_OK } else { NOTES_ANY };
    prop_oneof![
        6 => fnum(valid, lo, hi).prop_map(|x| json!(x)).boxed(),
        1 => proptest::sample::select(notes.to_vec()).prop_map(|s| json!(s)).boxed(),
        3 => modulator_json(valid),
    ]
    .boxed()
}

// --- Node generators -------------------------------------------------------

fn j_osc(valid: bool) -> SJ {
    (
        proptest::sample::select(vec!["sine", "triangle", "sawtooth"]),
        value_json(valid, 20.0, 10_000.0),
    )
        .prop_map(|(t, f)| json!({ "type": t, "freq": f }))
        .boxed()
}

fn j_square(valid: bool) -> SJ {
    (
        value_json(valid, 20.0, 10_000.0),
        value_json(valid, 0.0, 1.0),
    )
        .prop_map(|(f, d)| json!({ "type": "square", "freq": f, "duty": d }))
        .boxed()
}

fn j_noise() -> SJ {
    proptest::sample::select(vec!["white", "pink", "brown"])
        .prop_map(|c| json!({ "type": "noise", "color": c }))
        .boxed()
}

fn j_fm(valid: bool) -> SJ {
    (
        value_json(valid, 20.0, 10_000.0),
        fnum(valid, 0.25, 8.0),
        value_json(valid, 0.0, 8.0),
    )
        .prop_map(
            |(f, ratio, index)| json!({ "type": "fm", "freq": f, "ratio": ratio, "index": index }),
        )
        .boxed()
}

fn j_super(valid: bool) -> SJ {
    (
        proptest::sample::select(vec!["sawtooth", "square"]),
        value_json(valid, 20.0, 10_000.0),
        uint(valid, 1, 16, 0, 20),
        fnum(valid, 0.0, 12_000.0),
    )
        .prop_map(|(w, f, voices, detune)| {
            json!({ "type": "super", "wave": w, "freq": f, "voices": voices, "detune_cents": detune })
        })
        .boxed()
}

fn j_seq(valid: bool) -> SJ {
    let waves = if valid { WAVES_OK } else { WAVES_ANY };
    let n_notes = if valid { 1..=6usize } else { 0..=6usize };
    (
        fnum(valid, 40.0, 300.0),
        uint(valid, 1, 8, 0, 8),
        proptest::sample::select(waves.to_vec()),
        value_json(valid, 0.0, 1.0),
        fnum(valid, 0.0, 1.0),
        fnum(valid, 0.0, 1.0),
        adsr_json(valid),
        proptest::collection::vec(
            (
                0..16u32,
                uint(valid, 1, 8, 0, 17),
                value_json(valid, 20.0, 4_000.0),
                fnum(valid, 0.0, 1.0),
            )
                .prop_map(|(step, len, pitch, gain)| {
                    json!({ "step": step, "len": len, "pitch": pitch, "gain": gain })
                }),
            n_notes,
        ),
        // A couple of voice knobs, so knob validation gets exercised too.
        fnum(valid, 0.5, 4.0),
        fnum(valid, 0.8, 0.999),
        fnum(valid, 0.01, 0.5),
    )
        .prop_map(
            |(bpm, spb, wave, duty, swing, humanize, env, notes, fm_ratio, pluck_decay, bass_decay)| {
                with_fields(
                    &env,
                    &[
                        ("type", json!("seq")),
                        ("bpm", json!(bpm)),
                        ("steps_per_beat", json!(spb)),
                        ("wave", json!(wave)),
                        ("duty", duty),
                        ("swing", json!(swing)),
                        ("humanize", json!(humanize)),
                        ("notes", json!(notes)),
                        ("fm_ratio", json!(fm_ratio)),
                        ("pluck_decay", json!(pluck_decay)),
                        ("bass_decay", json!(bass_decay)),
                        ("sf2", json!("/nonexistent/fuzz.sf2")),
                    ],
                )
            },
        )
        .boxed()
}

fn j_impact(valid: bool) -> SJ {
    (fnum(valid, 0.0, 1.0), fnum(valid, 0.0, 1.0))
        .prop_map(|(h, v)| json!({ "type": "impact", "hardness": h, "velocity": v }))
        .boxed()
}

fn j_dust(valid: bool) -> SJ {
    (fnum(valid, 0.5, 400.0), fnum(valid, 0.0, 0.1))
        .prop_map(|(d, dec)| json!({ "type": "dust", "density": d, "decay": dec }))
        .boxed()
}

fn j_env(valid: bool) -> SJ {
    adsr_json(valid)
        .prop_map(|e| with_fields(&e, &[("type", json!("env"))]))
        .boxed()
}

fn source_json(valid: bool) -> SJ {
    prop_oneof![
        j_osc(valid),
        j_square(valid),
        j_noise(),
        j_fm(valid),
        j_super(valid),
        j_seq(valid),
        j_impact(valid),
        j_dust(valid),
        j_env(valid),
    ]
    .boxed()
}

fn j_filter(valid: bool) -> SJ {
    (
        proptest::sample::select(vec!["lowpass", "highpass", "bandpass", "notch"]),
        value_json(valid, 20.0, 10_000.0),
        fnum(valid, 0.05, 10.0),
    )
        .prop_map(|(t, c, q)| json!({ "type": t, "cutoff": c, "q": q }))
        .boxed()
}

fn j_eq(valid: bool) -> SJ {
    prop_oneof![
        (
            value_json(valid, 20.0, 10_000.0),
            fnum(valid, 0.05, 10.0),
            fnum(valid, -24.0, 24.0),
        )
            .prop_map(|(c, q, g)| json!({ "type": "peak", "cutoff": c, "q": q, "gain_db": g }))
            .boxed(),
        (
            proptest::sample::select(vec!["lowshelf", "highshelf"]),
            value_json(valid, 20.0, 10_000.0),
            fnum(valid, -24.0, 24.0),
        )
            .prop_map(|(t, c, g)| json!({ "type": t, "cutoff": c, "gain_db": g }))
            .boxed(),
    ]
    .boxed()
}

fn j_modal(valid: bool) -> SJ {
    let n = if valid { 1..=6usize } else { 0..=6usize };
    (
        proptest::collection::vec(
            (
                fnum(valid, 50.0, 4_000.0),
                fnum(valid, 0.01, 2.0),
                fnum(valid, 0.0, 1.0),
            )
                .prop_map(
                    |(freq, decay, gain)| json!({ "freq": freq, "decay": decay, "gain": gain }),
                ),
            n,
        ),
        fnum(valid, 0.0, 1.0),
    )
        .prop_map(|(modes, mix)| json!({ "type": "modal", "modes": modes, "mix": mix }))
        .boxed()
}

fn j_modfx(valid: bool) -> SJ {
    prop_oneof![
        (fnum(valid, 0.0, 40.0), fnum(valid, 0.0, 1.0))
            .prop_map(|(r, d)| json!({ "type": "tremolo", "rate": r, "depth": d }))
            .boxed(),
        (
            fnum(valid, 0.05, 20.0),
            fnum(valid, 0.0, 1.0),
            fnum(valid, 0.0, 1.0)
        )
            .prop_map(|(r, d, m)| json!({ "type": "chorus", "rate": r, "depth": d, "mix": m }))
            .boxed(),
        (
            proptest::sample::select(vec!["flanger", "phaser"]),
            fnum(valid, 0.05, 20.0),
            fnum(valid, 0.0, 1.0),
            fnum(valid, 0.0, 1.0),
            fnum(valid, 0.0, 1.0),
        )
            .prop_map(|(t, r, d, f, m)| {
                json!({ "type": t, "rate": r, "depth": d, "feedback": f, "mix": m })
            })
            .boxed(),
    ]
    .boxed()
}

fn j_tail_fx(valid: bool) -> SJ {
    prop_oneof![
        (
            fnum(valid, 0.05, 3.0),
            fnum(valid, 0.0, 3.0),
            fnum(valid, 0.0, 0.5),
            fnum(valid, 0.0, 1.0),
            fnum(valid, 0.0, 1.0),
        )
            .prop_map(|(decay, size, predelay, damp, mix)| {
                json!({ "type": "convolve", "decay": decay, "size": size, "predelay": predelay, "damp": damp, "mix": mix })
            })
            .boxed(),
        (
            fnum(valid, 5.0, 500.0),
            fnum(valid, 0.1, 200.0),
            fnum(valid, 0.25, 4.0),
            fnum(valid, 0.0, 1.0),
            fnum(valid, 0.0, 1.0),
        )
            .prop_map(|(grain_ms, density, pitch, spread, mix)| {
                json!({ "type": "granular", "grain_ms": grain_ms, "density": density, "pitch": pitch, "spread": spread, "mix": mix })
            })
            .boxed(),
    ]
    .boxed()
}

fn processor_json(valid: bool) -> SJ {
    prop_oneof![
        j_filter(valid),
        j_eq(valid),
        value_json(valid, -2.0, 2.0)
            .prop_map(|a| json!({ "type": "gain", "amount": a }))
            .boxed(),
        uint(valid, 1, 16, 0, 20)
            .prop_map(|b| json!({ "type": "bitcrush", "bits": b }))
            .boxed(),
        uint(valid, 1, 8, 0, 8)
            .prop_map(|f| json!({ "type": "downsample", "factor": f }))
            .boxed(),
        (fnum(valid, 0.001, 2.0), fnum(valid, 0.0, 1.0))
            .prop_map(|(s, f)| json!({ "type": "delay", "secs": s, "feedback": f }))
            .boxed(),
        (fnum(valid, 0.0, 1.0), fnum(valid, 0.0, 1.0))
            .prop_map(|(r, m)| json!({ "type": "reverb", "room": r, "mix": m }))
            .boxed(),
        j_modal(valid),
        (
            value_json(valid, 0.0, 8.0),
            proptest::sample::select(vec!["tanh", "hard", "fold"]),
        )
            .prop_map(|(a, s)| json!({ "type": "drive", "amount": a, "shape": s }))
            .boxed(),
        value_json(valid, 20.0, 10_000.0)
            .prop_map(|f| json!({ "type": "ringmod", "freq": f }))
            .boxed(),
        j_modfx(valid),
        j_tail_fx(valid),
        (
            fnum(valid, -60.0, 0.0),
            fnum(valid, 1.0, 20.0),
            fnum(valid, 0.0, 0.5),
            fnum(valid, 0.0, 0.5),
            fnum(valid, -12.0, 12.0),
        )
            .prop_map(|(t, r, a, rel, m)| {
                json!({ "type": "compress", "threshold": t, "ratio": r, "attack": a, "release": rel, "makeup": m })
            })
            .boxed(),
    ]
    .boxed()
}

fn leaf_json(valid: bool) -> SJ {
    prop_oneof![3 => source_json(valid), 2 => processor_json(valid)].boxed()
}

/// Any node graph, depth-capped so validation/render stay off the stack.
fn node_json(valid: bool, depth: u32) -> SJ {
    let leaf = leaf_json(valid);
    if depth == 0 {
        return leaf;
    }
    let inner = node_json(valid, depth - 1);
    prop_oneof![
        5 => leaf,
        1 => proptest::collection::vec(inner.clone(), 1..=3)
            .prop_map(|inputs| json!({ "type": "mix", "inputs": inputs }))
            .boxed(),
        1 => proptest::collection::vec(inner.clone(), 1..=3)
            .prop_map(|inputs| json!({ "type": "mul", "inputs": inputs }))
            .boxed(),
        1 => proptest::collection::vec(inner.clone(), 1..=3)
            .prop_map(|stages| json!({ "type": "chain", "stages": stages }))
            .boxed(),
        1 => (
            inner,
            fnum(valid, 0.0, 1.0),
            fnum(valid, 0.0, 0.1),
            fnum(valid, 0.0, 0.5),
        )
            .prop_map(|(t, amount, attack, release)| {
                json!({ "type": "duck", "trigger": t, "amount": amount, "attack": attack, "release": release })
            })
            .boxed(),
    ]
    .boxed()
}

fn lane_json(valid: bool, target: &'static str) -> SJ {
    let n = if valid { 1..=3usize } else { 0..=3usize };
    proptest::collection::vec(
        (fnum(valid, 0.0, 0.15), fnum(valid, -1.0, 2.0))
            .prop_map(|(t, v)| json!({ "t": t, "v": v })),
        n,
    )
    .prop_map(move |points| json!({ "target": target, "points": points }))
    .boxed()
}

fn automation_json(valid: bool) -> SJ {
    (
        proptest::option::of(lane_json(valid, "gain")),
        proptest::option::of(lane_json(valid, "pan")),
    )
        .prop_map(|(g, p)| J::Array([g, p].into_iter().flatten().collect()))
        .boxed()
}

fn track_json(valid: bool, depth: u32) -> SJ {
    let ids: Vec<Option<String>> = vec![
        None,
        Some("kick".into()),
        Some("bass".into()),
        Some("Bad ID".into()),
        Some("master".into()),
        Some(String::new()),
    ];
    (
        proptest::sample::select(ids),
        node_json(valid, depth),
        fnum(valid, -1.0, 1.0),
        fnum(valid, 0.0, 2.0),
        fnum(valid, 0.0, 0.04),
        any::<bool>(),
        automation_json(valid),
    )
        .prop_map(|(id, node, pan, gain, at, mute, automation)| {
            let mut o = serde_json::Map::new();
            if let Some(id) = id {
                o.insert("id".into(), id.into());
            }
            o.insert("node".into(), node);
            o.insert("pan".into(), pan.into());
            o.insert("gain".into(), gain.into());
            o.insert("at".into(), at.into());
            o.insert("mute".into(), mute.into());
            o.insert("automation".into(), automation);
            J::Object(o)
        })
        .boxed()
}

fn tracks_json(valid: bool, depth: u32) -> SJ {
    let n = if valid { 1..=3usize } else { 0..=3usize };
    (
        proptest::collection::vec(track_json(valid, depth), n),
        proptest::collection::vec(processor_json(valid), 0..=2),
    )
        .prop_map(move |(mut tracks, master)| {
            if valid {
                // Unique, well-formed ids so validish docs stay valid.
                for (i, t) in tracks.iter_mut().enumerate() {
                    if let Some(o) = t.as_object_mut() {
                        o.insert("id".into(), format!("layer_{i}").into());
                    }
                }
            }
            json!({ "type": "tracks", "tracks": tracks, "master": master })
        })
        .boxed()
}

// --- Document generator ----------------------------------------------------

fn duration_json(valid: bool) -> BoxedStrategy<f64> {
    if valid {
        (0.05..=0.15f64).boxed()
    } else {
        prop_oneof![
            4 => 0.05..=0.15f64,
            2 => proptest::sample::select(vec![0.0, -0.0, -1.0, 1e-38, 0.6, 600.0, 600.0001, 1e38]),
        ]
        .boxed()
    }
}

fn sample_rate_json(valid: bool) -> BoxedStrategy<u32> {
    if valid {
        (8_000..=24_000u32).boxed()
    } else {
        prop_oneof![
            4 => 8_000..=24_000u32,
            2 => proptest::sample::select(vec![0u32, 1, 7_999, 44_100, 192_000, 192_001, u32::MAX]),
        ]
        .boxed()
    }
}

fn stereo_json(valid: bool) -> SJ {
    let haas = (fnum(valid, 0.5, 40.0), fnum(valid, -1.0, 1.0))
        .prop_map(|(ms, pan)| json!({ "mode": "haas", "ms": ms, "pan": pan }))
        .boxed();
    let wide = fnum(valid, 0.0, 1.0)
        .prop_map(|amount| json!({ "mode": "wide", "amount": amount }))
        .boxed();
    prop_oneof![
        3 => Just(J::Null),
        1 => Just(json!({ "mode": "mono" })),
        1 => haas,
        1 => wide,
    ]
    .boxed()
}

fn normalize_json(valid: bool) -> SJ {
    prop_oneof![
        3 => Just(J::Null),
        2 => (fnum(valid, -60.0, 0.0), fnum(valid, -12.0, 0.0))
            .prop_map(|(t, c)| json!({ "target_lufs": t, "ceiling_dbtp": c }))
            .boxed(),
    ]
    .boxed()
}

fn playback_json(valid: bool) -> SJ {
    prop_oneof![
        3 => Just(J::Null),
        1 => Just(json!({ "mode": "oneshot" })),
        2 => (fnum(valid, 0.0, 0.04), fnum(valid, 0.001, 0.05))
            .prop_map(|(s, x)| json!({ "mode": "loop", "start_secs": s, "crossfade_secs": x }))
            .boxed(),
    ]
    .boxed()
}

fn corrupt_json() -> SJ {
    proptest::sample::select(vec![
        J::Null,
        json!([]),
        json!("nope"),
        json!(42),
        json!({ "root": 5 }),
        json!({ "name": [], "duration": "long", "root": J::Null }),
        json!({ "root": { "type": "unknown_kind" } }),
        json!({ "root": { "type": "sine", "freq": [1, 2, 3] } }),
    ])
    .boxed()
}

/// A full SoundDoc-shaped JSON value. `valid` biases parameters into their
/// validation domains (so property 2 and 4 see plenty of passing docs);
/// `!valid` also produces structurally corrupt JSON and out-of-domain values.
fn doc_json(valid: bool) -> SJ {
    let proper = (
        any::<u64>(),
        duration_json(valid),
        sample_rate_json(valid),
        proptest::option::of(proptest::sample::select(if valid {
            vec![tono_core::dsl::SCHEMA_VERSION]
        } else {
            vec![0, 1, 2, 3, 99]
        })),
        proptest::option::of(proptest::sample::select(if valid {
            vec![tono_core::dsl::ENGINE_VERSION]
        } else {
            vec![0, 1, 2, 3, 4, 5, 99]
        })),
        stereo_json(valid),
        normalize_json(valid),
        playback_json(valid),
        prop_oneof![7 => node_json(valid, 3), 3 => tracks_json(valid, 2)].boxed(),
    )
        .prop_map(
            |(seed, duration, sample_rate, version, engine, stereo, normalize, playback, root)| {
                let mut o = serde_json::Map::new();
                o.insert("name".into(), "fuzz".into());
                o.insert("duration".into(), duration.into());
                o.insert("sample_rate".into(), sample_rate.into());
                o.insert("seed".into(), seed.into());
                if let Some(v) = version {
                    o.insert("version".into(), v.into());
                }
                if let Some(e) = engine {
                    o.insert("engine".into(), e.into());
                }
                if !stereo.is_null() {
                    o.insert("stereo".into(), stereo);
                }
                if !normalize.is_null() {
                    o.insert("normalize".into(), normalize);
                }
                if !playback.is_null() {
                    o.insert("playback".into(), playback);
                }
                o.insert("root".into(), root);
                J::Object(o)
            },
        )
        .boxed();
    if valid {
        proper
    } else {
        prop_oneof![9 => proper, 1 => corrupt_json()].boxed()
    }
}

// --- Poison pass (NaN/±inf can't travel through JSON) ----------------------

fn poison_adsr(a: &mut Adsr, x: f32) {
    a.a = x;
    a.d = x;
    a.s = x;
    a.r = x;
    a.punch = x;
}

fn poison_value(v: &mut Value, x: f32) {
    match v {
        Value::Const(c) => *c = x,
        Value::Note(_) => {}
        Value::Modulated(m) => match m {
            Modulator::Slide { from, to, secs, .. } => {
                *from = x;
                *to = x;
                *secs = x;
            }
            Modulator::Lfo {
                rate,
                depth,
                center,
                ..
            } => {
                *rate = x;
                *depth = x;
                *center = x;
            }
            Modulator::Arp { steps, rate } => {
                steps.iter_mut().for_each(|s| *s = x);
                *rate = x;
            }
            Modulator::EnvMod { adsr, from, to } => {
                poison_adsr(adsr, x);
                *from = x;
                *to = x;
            }
            Modulator::Rand { from, to, rate, .. } => {
                *from = x;
                *to = x;
                *rate = x;
            }
            _ => {}
        },
    }
}

/// Overwrite every reachable f32 field of the graph with `x` — the doc stays
/// parsed (serde is done), but validate/render now face NaN/±inf/extremes.
fn poison_node(node: &mut Node, x: f32) {
    match node {
        Node::Square { freq, duty } => {
            poison_value(freq, x);
            poison_value(duty, x);
        }
        Node::Triangle { freq }
        | Node::Sawtooth { freq }
        | Node::Sine { freq }
        | Node::RingMod { freq } => poison_value(freq, x),
        Node::Fm { freq, ratio, index } => {
            poison_value(freq, x);
            *ratio = x;
            poison_value(index, x);
        }
        Node::Super {
            freq, detune_cents, ..
        } => {
            poison_value(freq, x);
            *detune_cents = x;
        }
        Node::Seq {
            bpm,
            duty,
            fm,
            pluck,
            piano,
            bass,
            swing,
            humanize,
            env,
            notes,
            ..
        } => {
            *bpm = x;
            poison_value(duty, x);
            fm.fm_ratio = x;
            fm.fm_index = x;
            fm.fm_strike = x;
            pluck.pluck_decay = x;
            pluck.pluck_body = x;
            pluck.pluck_pick = x;
            pluck.pluck_tone = x;
            piano.piano_hammer = x;
            piano.piano_strike = x;
            piano.piano_inharm = x;
            piano.piano_detune = x;
            piano.piano_decay = x;
            bass.bass_cutoff = x;
            bass.bass_env = x;
            bass.bass_env_vel = x;
            bass.bass_decay = x;
            bass.bass_click = x;
            bass.bass_body = x;
            bass.bass_sub = x;
            bass.bass_sub_ratio = x;
            bass.bass_drive = x;
            bass.bass_body_decay = x;
            *swing = x;
            *humanize = x;
            poison_adsr(env, x);
            for n in notes.iter_mut() {
                poison_value(&mut n.pitch, x);
                n.gain = x;
            }
        }
        Node::Impact { hardness, velocity } => {
            *hardness = x;
            *velocity = x;
        }
        Node::Dust { density, decay } => {
            *density = x;
            *decay = x;
        }
        Node::Env { adsr } => poison_adsr(adsr, x),
        Node::Lowpass { cutoff, q }
        | Node::Highpass { cutoff, q }
        | Node::Bandpass { cutoff, q }
        | Node::Notch { cutoff, q } => {
            poison_value(cutoff, x);
            *q = x;
        }
        Node::Peak { cutoff, q, gain_db } => {
            poison_value(cutoff, x);
            *q = x;
            *gain_db = x;
        }
        Node::Lowshelf { cutoff, gain_db } | Node::Highshelf { cutoff, gain_db } => {
            poison_value(cutoff, x);
            *gain_db = x;
        }
        Node::Gain { amount } | Node::Drive { amount, .. } => poison_value(amount, x),
        Node::Delay { secs, feedback } => {
            *secs = x;
            *feedback = x;
        }
        Node::Reverb { room, mix } => {
            *room = x;
            *mix = x;
        }
        Node::Modal { modes, mix } => {
            for m in modes.iter_mut() {
                m.freq = x;
                m.decay = x;
                m.gain = x;
            }
            *mix = x;
        }
        Node::Tremolo { rate, depth } => {
            *rate = x;
            *depth = x;
        }
        Node::Chorus { rate, depth, mix } => {
            *rate = x;
            *depth = x;
            *mix = x;
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
            *rate = x;
            *depth = x;
            *feedback = x;
            *mix = x;
        }
        Node::Duck {
            amount,
            attack,
            release,
            ..
        } => {
            *amount = x;
            *attack = x;
            *release = x;
        }
        Node::Compress {
            threshold,
            ratio,
            attack,
            release,
            makeup,
        } => {
            *threshold = x;
            *ratio = x;
            *attack = x;
            *release = x;
            *makeup = x;
        }
        Node::Convolve {
            decay,
            size,
            predelay,
            damp,
            mix,
        } => {
            *decay = x;
            *size = x;
            *predelay = x;
            *damp = x;
            *mix = x;
        }
        Node::Granular {
            grain_ms,
            density,
            pitch,
            spread,
            mix,
        } => {
            *grain_ms = x;
            *density = x;
            *pitch = x;
            *spread = x;
            *mix = x;
        }
        Node::Tracks { tracks, .. } => {
            for t in tracks.iter_mut() {
                t.pan = x;
                t.gain = x;
                t.at = x;
                for lane in t.automation.iter_mut() {
                    for p in lane.points.iter_mut() {
                        p.t = x;
                        p.v = x;
                    }
                }
                if let Some(sc) = &mut t.sidechain {
                    sc.amount = x;
                    sc.attack = x;
                    sc.release = x;
                }
            }
        }
        _ => {}
    }
}

fn poison_doc(doc: &mut SoundDoc, x: f32) {
    doc.root.walk_mut(&mut |n| poison_node(n, x));
    if let Some(nz) = &mut doc.normalize {
        if let Some(t) = &mut nz.target_lufs {
            *t = x;
        }
        nz.ceiling_dbtp = x;
    }
    match &mut doc.stereo {
        Stereo::Haas { ms, pan } => {
            *ms = x;
            *pan = x;
        }
        Stereo::Wide { amount } => *amount = x,
        _ => {}
    }
    if let Playback::Loop {
        start_secs,
        end_secs,
        crossfade_secs,
    } = &mut doc.playback
    {
        *start_secs = x;
        if let Some(e) = end_secs {
            *e = x;
        }
        *crossfade_secs = x;
    }
}

// --- The properties --------------------------------------------------------

/// Regression for a violation this suite found: an unvalidated `tracks`
/// document whose automation lane has exactly one point with a NaN time used
/// to panic the renderer — `lane_for` fell through both early-return
/// comparisons (NaN compares false) and indexed `pts[idx + 1]` on a
/// 1-element vec. Validation rejects the input ("automation[gain].points[0].t
/// must be >= 0 seconds, got NaN"), but the crate's contract is that an
/// UNVALIDATED document can't panic the renderer. serde_json can't carry NaN,
/// so the NaN form only arrives via a programmatically-built doc.
#[test]
fn single_point_nan_lane_renders_without_panic() {
    let mut doc: SoundDoc = serde_json::from_str(
        r#"{ "name": "repro", "duration": 0.05,
            "root": { "type": "tracks", "tracks": [
                { "id": "a", "node": { "type": "sine", "freq": 440 },
                  "automation": [ { "target": "gain", "points": [ { "t": 0.0, "v": 1.0 } ] } ] }
            ] } }"#,
    )
    .expect("repro doc parses");
    let Node::Tracks { tracks, .. } = &mut doc.root else {
        unreachable!("repro doc is a tracks doc")
    };
    tracks[0].automation[0].points[0].t = f32::NAN;
    assert!(
        doc.validate().is_err(),
        "validation must reject the NaN time"
    );
    // The unvalidated-render contract: no panic (a single breakpoint holds
    // flat — the only sane semantics, and now the guard).
    let product = render::render_product(&doc);
    assert!(product.mono.iter().all(|s| s.is_finite()));
}

/// The multi-point sibling of the single-point case above: a multi-breakpoint
/// lane on an unvalidated doc with `sample_rate` 0 made frame 0's time NaN
/// (0.0/0.0), which slipped past both flat-guards and indexed off the end of
/// the lane's segment scan. The cursor now holds the first point on a NaN
/// sample time (infinite times already take the last-point branch).
#[test]
fn multi_point_lane_at_zero_sample_rate_renders_without_panic() {
    let doc: SoundDoc = serde_json::from_str(
        r#"{ "name": "repro", "duration": 0.05, "sample_rate": 0,
            "root": { "type": "tracks", "tracks": [
                { "id": "a", "node": { "type": "sine", "freq": 440 },
                  "automation": [ { "target": "gain", "points": [
                      { "t": 0.0, "v": 0.5 }, { "t": 0.02, "v": 1.0 } ] } ] }
            ] } }"#,
    )
    .expect("repro doc parses");
    assert!(doc.validate().is_err(), "validation rejects sample_rate 0");
    let product = render::render_product(&doc);
    assert!(product.mono.iter().all(|s| s.is_finite()));
}

/// Regression for a violation this suite found: `vary::mutate`'s
/// multiplicative jitter (`v * (1 + rng.bi() * amount)`, up to ×2) overflowed
/// f32 to ±inf for any parameter validation bounds only from BELOW (e.g.
/// `modal.modes[].decay` — finite, positive, uncapped above), breaking its
/// documented "stays valid" promise. The jitter now clamps to `[min,
/// f32::MAX]`. Shrunk case: decay 3.4e38 validates; `mutate(doc, 0.7495734,
/// 0)` used to make it inf.
#[test]
fn mutate_clamps_uncapped_params_to_finite() {
    let doc: SoundDoc = serde_json::from_str(
        r#"{ "name": "repro", "duration": 0.05, "sample_rate": 8000,
            "root": { "type": "chain", "stages": [
                { "type": "impact", "hardness": 0.0, "velocity": 0.0 },
                { "type": "modal", "mix": 0.0,
                  "modes": [ { "freq": 50.0, "decay": 3.4e38, "gain": 0.0 } ] } ] } }"#,
    )
    .expect("repro doc parses");
    doc.validate()
        .expect("the extreme-but-finite decay validates (no upper bound)");
    let mutated = vary::mutate(&doc, 0.7495734, 0);
    assert!(
        mutated.validate().is_ok(),
        "mutate promised a still-valid doc, got: {:?}",
        mutated.validate().err()
    );
}

/// The render buffer length the engine will allocate (mirrors the clamps in
/// `render`/`render_tracks`), so the fuzz can cap its own work.
fn estimated_samples(doc: &SoundDoc) -> f64 {
    f64::from(doc.duration.clamp(0.0, 600.0)) * f64::from(doc.sample_rate)
}

fn assert_product_finite(
    p: &render::RenderProduct,
) -> Result<(), proptest::test_runner::TestCaseError> {
    prop_assert!(
        p.mono.iter().all(|s| s.is_finite()),
        "non-finite sample in the mono render"
    );
    if let Some((l, r)) = &p.stereo {
        prop_assert!(
            l.iter().all(|s| s.is_finite()) && r.iter().all(|s| s.is_finite()),
            "non-finite sample in the stereo bus"
        );
    }
    Ok(())
}

proptest! {
    #![proptest_config(config())]

    /// Contract 1: parse + validate never panic, on any JSON.
    #[test]
    fn parse_and_validate_never_panic(json in doc_json(false)) {
        if let Ok(doc) = serde_json::from_value::<SoundDoc>(json) {
            let _ = doc.validate();
        }
    }

    /// Contract 2: a validated document renders finite samples.
    #[test]
    fn validated_docs_render_finite_samples(json in doc_json(true)) {
        let Ok(doc) = serde_json::from_value::<SoundDoc>(json) else {
            return Ok(());
        };
        if doc.validate().is_err() {
            return Ok(());
        }
        prop_assume!(estimated_samples(&doc) <= 100_000.0);
        let product = render::render_product(&doc);
        assert_product_finite(&product)?;
    }

    /// Contract 3: an unvalidated document — a parsed doc with NaN/±inf or
    /// extreme values injected into every f32 field — still can't panic
    /// validate/render, and the output stage leaves every sample finite
    /// (scrubbed), whatever validation thinks of it.
    #[test]
    fn unvalidated_docs_render_without_panic_and_finite_output(
        json in doc_json(false),
        poison in proptest::sample::select(POISON.to_vec()),
    ) {
        let Ok(mut doc) = serde_json::from_value::<SoundDoc>(json) else {
            return Ok(());
        };
        prop_assume!(estimated_samples(&doc) <= 50_000.0);
        poison_doc(&mut doc, poison);
        let _ = doc.validate(); // must not panic, whichever way it decides
        let product = render::render_product(&doc); // must not panic
        assert_product_finite(&product)?;
    }

    /// Contract 4: vary::mutate on a valid doc always yields a still-valid
    /// doc (the documented promise), for a handful of seeds.
    #[test]
    fn mutate_preserves_validity(
        json in doc_json(true),
        amount in 0.0f32..=1.0,
        seeds in proptest::collection::vec(any::<u64>(), 3),
    ) {
        let Ok(doc) = serde_json::from_value::<SoundDoc>(json) else {
            return Ok(());
        };
        if doc.validate().is_err() {
            return Ok(());
        }
        for seed in seeds {
            let mutated = vary::mutate(&doc, amount, seed);
            prop_assert!(
                mutated.validate().is_ok(),
                "mutate(amount={amount}, seed={seed}) broke validity: {:?}",
                mutated.validate().err()
            );
        }
    }
}
