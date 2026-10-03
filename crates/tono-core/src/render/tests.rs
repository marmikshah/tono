use super::*;
use crate::dsl::DriveShape;
use crate::dsl::Modulator;
use crate::dsl::Stereo;
use crate::dsp::{loudness_lufs_gated, true_peak_oversampled};

fn doc(json: &str) -> SoundDoc {
    serde_json::from_str(json).expect("deserialize")
}

fn rms(s: &[f32]) -> f32 {
    (s.iter().map(|x| x * x).sum::<f32>() / s.len() as f32).sqrt()
}

/// The determinism invariant for automation: a constant-value gain lane
/// (every breakpoint = the static gain) renders byte-identically to a track
/// with no automation at all — proving the automated path matches the fast
/// path for constant values, so existing documents are unaffected.
#[test]
fn constant_gain_automation_is_byte_identical_to_static_gain() {
    let base = r#"{ "name":"t", "duration":0.4, "seed":1, "version":2,
            "root":{ "type":"tracks", "tracks":[ { "id":"a", "gain":0.8, "pan":-0.3,
              "node":{ "type":"mul", "inputs":[ {"type":"sine","freq":330},
                {"type":"env","a":0.01,"d":0.3,"s":0.4,"r":0.05} ] } } ] } }"#;
    let auto = r#"{ "name":"t", "duration":0.4, "seed":1, "version":2,
            "root":{ "type":"tracks", "tracks":[ { "id":"a", "gain":0.8, "pan":-0.3,
              "automation":[{"target":"gain","points":[{"t":0,"v":0.8},{"t":0.4,"v":0.8}]}],
              "node":{ "type":"mul", "inputs":[ {"type":"sine","freq":330},
                {"type":"env","a":0.01,"d":0.3,"s":0.4,"r":0.05} ] } } ] } }"#;
    let a = render_tracks(&doc(base)).unwrap();
    let b = render_tracks(&doc(auto)).unwrap();
    let bits = |s: &[f32]| s.iter().map(|x| x.to_bits()).collect::<Vec<_>>();
    assert_eq!(bits(&a.left), bits(&b.left), "left byte-identical");
    assert_eq!(bits(&a.right), bits(&b.right), "right byte-identical");
}

/// A gain ramp from 1 → 0 over the document makes the second half quieter
/// than the first — automation actually rides the level.
#[test]
fn gain_automation_ramp_fades_the_track() {
    let d = doc(r#"{ "name":"t", "duration":1.0, "seed":1, "version":2,
            "root":{ "type":"tracks", "tracks":[ { "id":"a", "gain":1.0,
              "automation":[{"target":"gain","points":[{"t":0,"v":1.0},{"t":1.0,"v":0.0}]}],
              "node":{ "type":"sine", "freq":220 } } ] } }"#);
    let r = render_tracks(&d).unwrap();
    let half = r.left.len() / 2;
    let head = rms(&r.left[..half]);
    let tail = rms(&r.left[half..]);
    assert!(tail < head * 0.6, "ramp fades: head {head}, tail {tail}");
}

/// The default (no `curve` field) is linear, byte-for-byte: explicit
/// `"curve":"linear"` renders identically to no curve at all.
#[test]
fn automation_curve_defaults_to_linear_byte_identically() {
    let base = r#"{ "name":"t", "duration":0.4, "seed":1, "version":2,
            "root":{ "type":"tracks", "tracks":[ { "id":"a", "gain":1.0,
              "automation":[{"target":"gain","points":[{"t":0,"v":0.5},{"t":0.4,"v":1.0}]}],
              "node":{ "type":"sine", "freq":220 } } ] } }"#;
    let explicit = r#"{ "name":"t", "duration":0.4, "seed":1, "version":2,
            "root":{ "type":"tracks", "tracks":[ { "id":"a", "gain":1.0,
              "automation":[{"target":"gain","curve":"linear","points":[{"t":0,"v":0.5},{"t":0.4,"v":1.0}]}],
              "node":{ "type":"sine", "freq":220 } } ] } }"#;
    let a = render_tracks(&doc(base)).unwrap();
    let b = render_tracks(&doc(explicit)).unwrap();
    let bits = |s: &[f32]| s.iter().map(|x| x.to_bits()).collect::<Vec<_>>();
    assert_eq!(bits(&a.left), bits(&b.left));
}

/// A step lane holds the old value until the breakpoint, then jumps — no
/// ramp. The midpoint before the jump measures the old level; after, the
/// new one.
#[test]
fn automation_step_holds_then_jumps() {
    let d = doc(r#"{ "name":"t", "duration":1.0, "seed":1, "version":2,
            "root":{ "type":"tracks", "tracks":[ { "id":"a", "gain":1.0,
              "automation":[{"target":"gain","curve":"step","points":[{"t":0,"v":0.2},{"t":0.5,"v":1.0}]}],
              "node":{ "type":"sine", "freq":220 } } ] } }"#);
    let r = render_tracks(&d).unwrap();
    let sr_len = r.left.len();
    let q1 = rms(&r.left[..sr_len / 4]); // 0..0.25s — held at 0.2
    let q3 = rms(&r.left[sr_len / 2..sr_len * 3 / 4]); // 0.5..0.75s — jumped to 1.0
    let ratio = q3 / q1;
    assert!(
        ratio > 4.0,
        "step jumps 0.2 → 1.0 with no ramp: {q1} vs {q3}"
    );
}

/// An exp lane between positive endpoints is geometric: the temporal
/// midpoint lands at the geometric mean (√(v0·v1)), not the arithmetic one.
#[test]
fn automation_exp_is_geometric_between_positive_endpoints() {
    // v0 = 0.1, v1 = 0.4, linear midpoint 0.25 vs geometric 0.2 — 25% apart.
    let mk = |curve: &str| {
        doc(&format!(
            r#"{{ "name":"t", "duration":1.0, "seed":1, "version":2,
                "root":{{ "type":"tracks", "tracks":[ {{ "id":"a", "gain":1.0,
                  "automation":[{{"target":"gain","curve":"{curve}","points":[{{"t":0,"v":0.1}},{{"t":1.0,"v":0.4}}]}}],
                  "node":{{ "type":"sine", "freq":220 }} }} ] }} }}"#
        ))
    };
    let lin = render_tracks(&mk("linear")).unwrap();
    let exp = render_tracks(&mk("exp")).unwrap();
    let n = lin.left.len();
    // The window 0.45..0.55 s centers on the midpoint.
    let mid = |s: &[f32]| rms(&s[n * 45 / 100..n * 55 / 100]);
    let (l, e) = (mid(&lin.left), mid(&exp.left));
    assert!(
        (e / l - 0.8).abs() < 0.05,
        "geometric midpoint is 0.8× the linear one: {l} vs {e}"
    );
}

#[test]
fn render_product_mid_is_the_track_bus_average() {
    let d = doc(r#"{ "name": "t", "duration": 0.05, "seed": 3, "version": 2,
                 "root": { "type": "tracks", "tracks": [
                    { "node": { "type": "sine", "freq": 220 }, "gain": 0.5 },
                    { "node": { "type": "noise" }, "gain": 0.5, "pan": 0.5 }
                 ] } }"#);
    let p = render_product(&d);
    let (l, r) = p.stereo.as_ref().expect("tracks doc carries the bus");
    assert_eq!(p.mono.len(), l.len());
    for i in [0usize, 100, 1000] {
        assert_eq!(p.mono[i], 0.5 * (l[i] + r[i]));
    }
    // Plain documents carry no pair; stereo treatment happens at write time.
    let plain =
        doc(r#"{ "name": "p", "duration": 0.05, "root": { "type": "sine", "freq": 220 } }"#);
    assert!(render_product(&plain).stereo.is_none());
}

#[test]
fn tracks_have_independent_rng_streams() {
    // Two docs that differ ONLY in track 0 (a sine consumes no RNG draws, a
    // noise consumes one per sample). Track 1 is hard-panned right, so the
    // right channel is its noise alone. Gains stay at 0.5 so the joint peak
    // limit never engages and the channels compare bit-for-bit.
    let mk = |first: &str, version: &str| {
        doc(&format!(
            r#"{{ "name": "t", "duration": 0.05, "seed": 7{version},
                     "root": {{ "type": "tracks", "tracks": [
                        {{ "node": {first}, "pan": -1.0, "gain": 0.5 }},
                        {{ "node": {{ "type": "noise" }}, "pan": 1.0, "gain": 0.5 }}
                     ] }} }}"#
        ))
    };
    let right = |d: &SoundDoc| render_tracks(d).unwrap().right;
    let sine = r#"{ "type": "sine", "freq": 440 }"#;
    let noise = r#"{ "type": "noise" }"#;
    // v2: editing track 0 never changes track 1's noise content.
    assert_eq!(
        right(&mk(sine, r#", "version": 2"#)),
        right(&mk(noise, r#", "version": 2"#))
    );
    assert_eq!(right(&mk(sine, "")), right(&mk(noise, "")));
}

#[test]
fn layer_at_offset_shifts_and_truncates() {
    let d = doc(r#"{ "name": "t", "duration": 0.1, "version": 2,
                 "root": { "type": "tracks", "tracks": [
                    { "id": "late", "node": { "type": "sine", "freq": 440 },
                      "gain": 0.5, "at": 0.05 }
                 ] } }"#);
    let l = render_tracks(&d).unwrap().left;
    let head = rms(&l[..2000]);
    let tail = rms(&l[2300..]);
    assert!(head < 1e-6, "before `at` the bus is silent, rms {head}");
    assert!(tail > 0.1, "the layer plays from `at` on, rms {tail}");
    assert_eq!(l.len(), 4410); // shifted tail truncated at the doc edge
}

#[test]
fn muted_layer_is_exactly_absent() {
    let with_muted = doc(r#"{ "name": "t", "duration": 0.05, "seed": 9, "version": 2,
                 "root": { "type": "tracks", "tracks": [
                    { "id": "keep", "node": { "type": "noise" }, "gain": 0.5 },
                    { "id": "gone", "node": { "type": "noise" }, "gain": 0.5, "mute": true }
                 ] } }"#);
    let without = doc(r#"{ "name": "t", "duration": 0.05, "seed": 9, "version": 2,
                 "root": { "type": "tracks", "tracks": [
                    { "id": "keep", "node": { "type": "noise" }, "gain": 0.5 }
                 ] } }"#);
    // Id-keyed streams: muting/removing a sibling never re-grains "keep".
    let (a, b) = (
        render_tracks(&with_muted).unwrap(),
        render_tracks(&without).unwrap(),
    );
    assert_eq!((a.left, a.right), (b.left, b.right));
    // The muted layer still reports a (silent) stats row.
    assert!(a.layers[1].mute && a.layers[1].energy_pct == 0.0);
}

#[test]
fn layer_stats_report_contribution() {
    let d = doc(r#"{ "name": "t", "duration": 0.05, "version": 2,
                 "root": { "type": "tracks", "tracks": [
                    { "id": "loud", "node": { "type": "sine", "freq": 220 }, "gain": 0.8 },
                    { "id": "quiet", "node": { "type": "sine", "freq": 330 }, "gain": 0.2 }
                 ] } }"#);
    let tr = render_tracks(&d).unwrap();
    assert_eq!(tr.layers.len(), 2);
    assert_eq!(tr.layers[0].id, "loud");
    assert_eq!(tr.layers[1].id, "quiet");
    // 0.8 vs 0.2 gain ⇒ a 16:1 energy split.
    assert!(tr.layers[0].energy_pct > 90.0, "{:?}", tr.layers);
    assert!(tr.layers[1].energy_pct < 10.0, "{:?}", tr.layers);
    let total: f32 = tr.layers.iter().map(|l| l.energy_pct).sum();
    assert!((total - 100.0).abs() < 0.1);
    // dB sanity: the loud layer peaks ~12 dB above the quiet one.
    let gap = tr.layers[0].peak_dbfs - tr.layers[1].peak_dbfs;
    assert!((gap - 12.04).abs() < 0.2, "gap {gap}");
}

#[test]
fn wrapping_a_plain_root_as_a_compensated_layer_is_level_neutral() {
    let plain = doc(r#"{ "name": "p", "duration": 0.05,
                 "root": { "type": "mul", "inputs": [
                    { "type": "sine", "freq": 330 },
                    { "type": "env", "a": 0.0, "d": 0.04, "s": 0.0, "r": 0.0 } ] } }"#);
    let wrapped = doc(r#"{ "name": "p", "duration": 0.05, "version": 2,
                 "root": { "type": "tracks", "tracks": [
                    { "id": "p", "gain": 1.4142135,
                      "node": { "type": "mul", "inputs": [
                        { "type": "sine", "freq": 330 },
                        { "type": "env", "a": 0.0, "d": 0.04, "s": 0.0, "r": 0.0 } ] } }
                 ] } }"#);
    let a = render(&plain);
    let b = render(&wrapped); // mid of the wrapped bus
    let max_diff = a
        .iter()
        .zip(&b)
        .fold(0.0f32, |m, (x, y)| m.max((x - y).abs()));
    assert!(
        max_diff < 1e-6,
        "wrap must be level-neutral, diff {max_diff}"
    );
}

#[test]
fn render_is_deterministic() {
    let d = doc(r#"{ "name": "n", "duration": 0.1, "seed": 7,
                 "root": { "type": "noise" } }"#);
    assert_eq!(render(&d), render(&d)); // same graph + seed ⇒ same bytes
    let mut d2 = d.clone();
    d2.seed = 8;
    assert_ne!(render(&d), render(&d2)); // different seed ⇒ different noise
}

#[test]
fn sine_has_expected_length_and_level() {
    let d = doc(r#"{ "name": "n", "duration": 0.1,
                 "root": { "type": "sine", "freq": 440 } }"#);
    let s = render(&d);
    assert_eq!(s.len(), 4410);
    let peak = s.iter().fold(0.0f32, |m, &x| m.max(x.abs()));
    assert!(peak > 0.9 && peak <= 1.0);
    assert!((rms(&s) - 0.7).abs() < 0.05); // sine RMS = peak/√2
}

#[test]
fn envelope_gates_the_oscillator() {
    let d = doc(
        r#"{ "name": "n", "duration": 0.2, "root": { "type": "mul", "inputs": [
                { "type": "square", "freq": 440 },
                { "type": "env", "a": 0.0, "d": 0.05, "s": 0.0, "r": 0.0 }
            ] } }"#,
    );
    let s = render(&d);
    // Loud at the start, silent at the end (envelope fully decayed).
    let head = rms(&s[..2205]);
    let tail = rms(&s[s.len() - 2205..]);
    assert!(head > 0.1, "head should be audible, rms {head}");
    assert!(tail < 1e-3, "tail should be silent, rms {tail}");
}

#[test]
fn slide_descends_pitch() {
    // A 880→110 Hz exponential slide: zero crossings in the first half
    // outnumber those in the second half.
    let d = doc(r#"{ "name": "n", "duration": 0.5, "root": { "type": "sine",
                 "freq": { "slide": { "from": 880, "to": 110, "secs": 0.5, "curve": "exp" } } } }"#);
    let s = render(&d);
    let crossings = |w: &[f32]| w.windows(2).filter(|p| p[0] * p[1] < 0.0).count();
    let (a, b) = s.split_at(s.len() / 2);
    assert!(crossings(a) > crossings(b) * 2);
}

#[test]
fn seq_places_notes_on_the_grid() {
    // 120 bpm, 4 steps/beat ⇒ 0.125 s per step. A note at step 2 starts at
    // 0.25 s; everything before is silence.
    let d = doc(r#"{ "name": "n", "duration": 0.6, "root": { "type": "seq",
                 "bpm": 120, "wave": "square",
                 "env": { "d": 0.1 },
                 "notes": [ { "step": 2, "len": 2, "pitch": "C4" } ] } }"#);
    let s = render(&d);
    let pre = rms(&s[..(0.24 * 44_100.0) as usize]);
    let post = rms(&s[(0.26 * 44_100.0) as usize..(0.35 * 44_100.0) as usize]);
    assert!(pre < 1e-4, "before the note: silence, rms {pre}");
    assert!(post > 0.05, "during the note: audible, rms {post}");
}

/// Brightness proxy: energy of the first difference relative to the
/// signal (high-frequency content differentiates to larger steps).
fn brightness(s: &[f32]) -> f32 {
    let diff: f32 = s.windows(2).map(|w| (w[1] - w[0]).powi(2)).sum();
    let total: f32 = s.iter().map(|x| x * x).sum();
    diff / total.max(1e-12)
}

#[test]
fn lowpass_darkens_highpass_brightens() {
    let noise = r#"{ "type": "noise" }"#;
    let plain = doc(&format!(
        r#"{{ "name": "n", "duration": 0.2, "root": {noise} }}"#
    ));
    let lp = doc(&format!(
        r#"{{ "name": "n", "duration": 0.2, "root": {{ "type": "chain", "stages": [
                {noise}, {{ "type": "lowpass", "cutoff": 500 }} ] }} }}"#
    ));
    let hp = doc(&format!(
        r#"{{ "name": "n", "duration": 0.2, "root": {{ "type": "chain", "stages": [
                {noise}, {{ "type": "highpass", "cutoff": 5000 }} ] }} }}"#
    ));
    let b_plain = brightness(&render(&plain));
    assert!(brightness(&render(&lp)) < b_plain * 0.5, "lowpass darkens");
    assert!(
        brightness(&render(&hp)) > b_plain * 1.1,
        "highpass brightens"
    );
}

#[test]
fn chain_processors_transform_in_series() {
    // sine → gain 0.25: the processor scales the running signal.
    let d = doc(
        r#"{ "name": "n", "duration": 0.05, "root": { "type": "chain", "stages": [
                { "type": "sine", "freq": 440 },
                { "type": "gain", "amount": 0.25 }
            ] } }"#,
    );
    let s = render(&d);
    let peak = s.iter().fold(0.0f32, |m, &x| m.max(x.abs()));
    assert!((peak - 0.25).abs() < 0.01);
}

#[test]
fn tremolo_at_full_depth_modulates_the_envelope() {
    // depth 1 swings the gain between 0 and 1 at `rate` Hz: windows near the
    // trough are almost silent, windows near the peak stay at full level.
    let d = doc(
        r#"{ "name": "n", "duration": 1.0, "root": { "type": "chain", "stages": [
                { "type": "sine", "freq": 440 },
                { "type": "tremolo", "rate": 5.0, "depth": 1.0 }
            ] } }"#,
    );
    let s = render(&d);
    let mut min = f32::INFINITY;
    let mut max = 0.0f32;
    // 1102 samples ≈ 0.025 s — an eighth of the 5 Hz tremolo period.
    for chunk in s.chunks(1102) {
        let r = rms(chunk);
        min = min.min(r);
        max = max.max(r);
    }
    assert!(max > 0.5, "peak windows stay loud, max window rms {max}");
    assert!(
        min < 0.1,
        "trough windows nearly silent, min window rms {min}"
    );
}

#[test]
fn tremolo_at_zero_depth_is_a_transparent_passthrough() {
    // gain = 1 - 0 * lfo = 1.0 exactly, so a chain with a depth-0 tremolo
    // renders byte-identically to the bare source.
    let plain = doc(r#"{ "name": "n", "duration": 0.2, "seed": 3,
                 "root": { "type": "sine", "freq": 440 } }"#);
    let through = doc(r#"{ "name": "n", "duration": 0.2, "seed": 3,
                 "root": { "type": "chain", "stages": [
                     { "type": "sine", "freq": 440 },
                     { "type": "tremolo", "rate": 6.0, "depth": 0.0 }
                 ] } }"#);
    let bits = |s: &[f32]| s.iter().map(|x| x.to_bits()).collect::<Vec<_>>();
    assert_eq!(bits(&render(&plain)), bits(&render(&through)));
}

#[test]
fn bitcrush_quantizes_amplitude() {
    // The gain stage keeps the crushed peak under the output ceiling so the
    // safety limit stays out of the way and the levels survive untouched.
    let d = doc(
        r#"{ "name": "n", "duration": 0.05, "root": { "type": "chain", "stages": [
                { "type": "sine", "freq": 100 },
                { "type": "gain", "amount": 0.5 },
                { "type": "bitcrush", "bits": 2 }
            ] } }"#,
    );
    let s = render(&d);
    // 2 bits ⇒ amplitudes land on multiples of 0.5.
    for x in &s {
        let nearest = (x / 0.5).round() * 0.5;
        assert!((x - nearest).abs() < 1e-4, "{x} not on a 2-bit level");
    }
}

#[test]
fn drive_hard_clips_to_unit_range() {
    let d = doc(
        r#"{ "name": "n", "duration": 0.05, "root": { "type": "chain", "stages": [
                { "type": "sine", "freq": 440 },
                { "type": "drive", "amount": 10, "shape": "hard" }
            ] } }"#,
    );
    // Heavy drive into a hard clip ⇒ near-square at the ceiling.
    let s = render(&d);
    let clipped = s.iter().filter(|x| x.abs() > 0.95).count();
    assert!(clipped > s.len() / 2);
}

#[test]
fn drive_fold_terminates_on_any_input() {
    // The fold loop runs per sample on the real-time path: a non-finite
    // input must not hang it, and huge amplitudes must stay bounded.
    assert_eq!(drive_curve(f32::NAN, DriveShape::Fold), 0.0);
    assert_eq!(drive_curve(f32::INFINITY, DriveShape::Fold), 0.0);
    assert_eq!(drive_curve(f32::NEG_INFINITY, DriveShape::Fold), 0.0);
    assert!((-1.0..=1.0).contains(&drive_curve(1.0e9, DriveShape::Fold)));
    // Realistic inputs keep the exact reflection behavior.
    assert_eq!(drive_curve(1.5, DriveShape::Fold), 0.5);
    assert_eq!(drive_curve(-1.5, DriveShape::Fold), -0.5);
}

#[test]
fn drive_antiderivative_matches_its_curve() {
    // F'(x) ≈ drive_curve(x): a central difference of the antiderivative
    // must reproduce the waveshaper, the property ADAA relies on.
    let h = 1e-3f32;
    for shape in [DriveShape::Tanh, DriveShape::Hard, DriveShape::Fold] {
        for &x in &[-3.5f32, -1.2, -0.4, 0.0, 0.6, 1.5, 4.2] {
            let num = (drive_antideriv(x + h, shape) - drive_antideriv(x - h, shape)) / (2.0 * h);
            let exact = drive_curve(x, shape);
            assert!(
                (num - exact).abs() < 5e-3,
                "{shape:?} at x={x}: dF/dx={num} vs f={exact}"
            );
        }
    }
}

#[test]
fn adaa_can_be_disabled_explicitly() {
    let mk = |aa: bool| {
        doc(&format!(
            r#"{{"name":"fold", "duration":0.1, "root":{{"type":"chain", "stages":[{{"type":"sine", "freq":3000}}, {{"type":"drive", "amount":6, "shape":"fold", "aa":{aa}}}]}}}}"#
        ))
    };
    let raw = render(&mk(false));
    let aa = render(&mk(true));
    let sine = render_node(
        &Node::Sine {
            freq: Value::Const(3000.0),
        },
        raw.len(),
        44_100,
        0,
    );
    let mut reference: Vec<f32> = sine
        .iter()
        .map(|x| drive_curve(6.0 * x, DriveShape::Fold))
        .collect();
    peak_limit(&mut [&mut reference]);
    assert_eq!(raw, reference, "aa:false uses the pointwise curve");
    assert_ne!(raw, aa);
    let diff_energy =
        |s: &[f32]| s.windows(2).map(|w| (w[1] - w[0]).powi(2)).sum::<f32>() / s.len() as f32;
    assert!(
        diff_energy(&aa) < diff_energy(&raw),
        "ADAA reduces high-frequency foldback"
    );
}

#[cfg(feature = "analysis")]
#[test]
fn adaa_lowers_off_harmonic_energy_for_a_folded_tone() {
    let mk = |aa: bool| {
        doc(&format!(
            r#"{{"name":"fold", "duration":0.3, "root":{{"type":"chain", "stages":[{{"type":"sine", "freq":2500}}, {{"type":"drive", "amount":8, "shape":"fold", "aa":{aa}}}]}}}}"#
        ))
    };
    let inharm = |d: &SoundDoc| crate::analysis::stats(&render(d), 44_100).inharmonicity;
    assert!(inharm(&mk(true)) < inharm(&mk(false)) - 0.1);
}

#[test]
fn impact_is_a_short_unit_area_pulse() {
    let d = doc(r#"{ "name": "n", "duration": 0.2, "engine": 5,
                 "root": { "type": "impact", "hardness": 0.5, "velocity": 1.0 } }"#);
    let s = render(&d);
    // The pulse is confined to the first ~10 ms; the rest is silence.
    let head = (0.02 * 44_100.0) as usize;
    assert!(
        s[head..].iter().all(|x| x.abs() < 1e-6),
        "impact must be a short burst"
    );
    // Unit area (× velocity 1) — the level guarantee a modal bank rings to.
    let area: f32 = s.iter().sum();
    assert!(
        (area - 1.0).abs() < 0.05,
        "impact area ≈ velocity, got {area}"
    );
}

#[test]
fn modal_bank_rings_at_its_mode_and_decays() {
    let d = doc(r#"{ "name": "n", "duration": 0.4, "engine": 5,
                 "root": { "type": "chain", "stages": [
                    { "type": "impact", "hardness": 0.8, "velocity": 1.0 },
                    { "type": "modal", "modes": [ { "freq": 1000, "decay": 0.3, "gain": 1.0 } ] }
                 ] } }"#);
    let s = render(&d);
    // Usable level (not the −44 dBFS the un-normalised first cut produced).
    let peak = s.iter().fold(0.0f32, |m, x| m.max(x.abs()));
    assert!(peak > 0.1, "modal ring too quiet: peak {peak}");
    // Rings at the mode frequency: count zero crossings over a steady
    // window and convert to Hz (a single mode is a clean decaying sine).
    let (a, b) = ((0.05 * 44_100.0) as usize, (0.15 * 44_100.0) as usize);
    let win = &s[a..b];
    let zc = win
        .windows(2)
        .filter(|w| (w[0] <= 0.0) != (w[1] <= 0.0))
        .count();
    let hz = zc as f32 / 2.0 / 0.1;
    assert!((hz - 1000.0).abs() < 80.0, "expected ≈1000 Hz, got {hz}");
    // Decays: the tail is quieter than the body.
    assert!(
        rms(&s[s.len() / 2..]) < rms(&s[..s.len() / 2]),
        "modal must decay"
    );
}

#[test]
fn rand_modulator_is_self_seeded_and_bounded() {
    let v = |seed: u64| {
        Value::Modulated(Modulator::Rand {
            from: 200.0,
            to: 800.0,
            rate: 5.0,
            seed,
        })
    };
    // Deterministic from its own fields only — no shared-stream coupling,
    // so a sibling edit elsewhere in the graph can never shift it.
    let a = eval_value(&v(1), 4410, 44_100);
    assert_eq!(a, eval_value(&v(1), 4410, 44_100));
    // A different seed decorrelates the walk.
    assert_ne!(a, eval_value(&v(2), 4410, 44_100));
    // The walk stays inside [from, to].
    assert!(a.iter().all(|&x| (200.0..=800.0).contains(&x)));
}

#[test]
fn dust_is_sparse_and_deterministic() {
    let mk = || {
        doc(r#"{ "name": "n", "duration": 1.0, "engine": 5, "seed": 4,
                     "root": { "type": "dust", "density": 20, "decay": 0.0 } }"#)
    };
    let a = render(&mk());
    assert_eq!(a, render(&mk()), "dust must be deterministic");
    // ~20 events/sec over 1 s; decay 0 ⇒ one nonzero sample per event.
    let events = a.iter().filter(|&&x| x.abs() > 1e-6).count();
    assert!(
        (5..60).contains(&events),
        "expected ≈20 sparse events, got {events}"
    );
}

#[test]
fn compressor_attenuates_above_threshold() {
    // A 0 dBFS sine through threshold −20 dB, ratio 4:1 settles at a steady
    // gain of −(0 − (−20))·(1 − 1/4) = −15 dB.
    let wet = doc(
        r#"{ "name": "n", "duration": 0.3, "root": { "type": "chain", "stages": [
                { "type": "sine", "freq": 440 },
                { "type": "compress", "threshold": -20, "ratio": 4 }
            ] } }"#,
    );
    let dry = doc(r#"{ "name": "n", "duration": 0.3, "root": { "type": "sine", "freq": 440 } }"#);
    // Skip the attack transient, measure the settled tail.
    let tail = |s: Vec<f32>| rms(&s[s.len() / 2..]);
    let ratio = tail(render(&wet)) / tail(render(&dry));
    let db = 20.0 * ratio.log10();
    assert!((db + 15.0).abs() < 2.0, "expected ≈ −15 dB, got {db:.1} dB");
}

#[test]
fn loop_body_is_region_minus_crossfade() {
    let sr = 1000u32;
    let samples = vec![0.5f32; 1000]; // 1 s
    // Region [0.2, 0.8) = 600 samples, crossfade 0.1 s = 100 ⇒ body 500.
    let out = make_loop_buffer(&samples, sr, 0.2, Some(0.8), 0.1);
    assert_eq!(out.len(), 500);
    // Degenerate inputs fall back gracefully.
    assert_eq!(
        make_loop_buffer(&samples, sr, 0.9, Some(0.1), 0.1).len(),
        1000
    );
    assert_eq!(make_loop_buffer(&samples, sr, 0.0, None, 0.0).len(), 1000);
}

#[test]
fn looped_render_has_a_quiet_seam() {
    // A sustained noise bed rendered as a loop: the wrap-around jump should
    // be far below the raw signal's sample-to-sample movement.
    let d = doc(r#"{ "name": "n", "duration": 1.0, "seed": 3,
                 "playback": { "mode": "loop", "crossfade_secs": 0.25 },
                 "root": { "type": "chain", "stages": [
                    { "type": "noise" }, { "type": "lowpass", "cutoff": 800 } ] } }"#);
    let s = render(&d);
    assert!(s.len() < 44_100); // body shortened by the crossfade
    assert!(loop_seam_db(&s) < -20.0, "seam {} dB", loop_seam_db(&s));
}

#[test]
fn normalize_hits_the_loudness_target() {
    let d = doc(r#"{ "name": "n", "duration": 0.5,
                 "normalize": { "target_lufs": -20, "ceiling_dbtp": -1 },
                 "root": { "type": "chain", "stages": [
                    { "type": "sine", "freq": 440 }, { "type": "gain", "amount": 0.05 } ] } }"#);
    let s = render(&d);
    let lufs = loudness_lufs_gated(&[&s], d.sample_rate);
    assert!((lufs + 20.0).abs() < 1.5, "got {lufs} LUFS");
    // True peak respects the −1 dBTP ceiling (small estimation slack).
    assert!(crate::dsp::dbfs(true_peak_oversampled(&s)) <= -0.9);
}

#[test]
fn humanize_jitters_chord_notes_independently() {
    let mk = |pitch: &str| {
        doc(&format!(
            r#"{{"name":"h", "duration":2, "root":{{"type":"seq", "wave":"sine", "bpm":120, "humanize":1, "env":{{"a":0.001, "d":0.01, "s":1, "r":0.01}}, "notes":[{{"step":4, "len":1, "pitch":"{pitch}"}}]}}}}"#
        ))
    };
    let onset = |d: &SoundDoc| render(d).iter().position(|x| x.abs() > 0.01).unwrap() as i64;
    assert!(
        (onset(&mk("C4")) - onset(&mk("E4"))).abs() > 8,
        "chord pitches get independent timing"
    );
    assert_eq!(
        render(&mk("C4")),
        render(&mk("C4")),
        "jitter stays deterministic"
    );
}

#[test]
fn normalize_preserves_the_stereo_balance() {
    let dry = doc(
        r#"{"name":"balance", "duration":1, "root":{"type":"tracks", "tracks":[{"id":"lead", "node":{"type":"sine", "freq":440}, "gain":0.1, "pan":0.6}]}}"#,
    );
    let mut normalized = dry.clone();
    normalized.normalize = Some(crate::dsl::Normalize {
        target_lufs: Some(-20.0),
        ceiling_dbtp: -1.0,
    });
    let ratio = |d: &SoundDoc| {
        let p = render_tracks(d).unwrap();
        rms(&p.left) / rms(&p.right)
    };
    assert!(
        (ratio(&dry) - ratio(&normalized)).abs() < 1e-4,
        "joint gain preserves the authored balance"
    );
}

#[test]
fn stereoize_modes_behave() {
    let d = doc(r#"{ "name": "n", "duration": 0.1, "root": { "type": "noise" } }"#);
    let mono = render(&d);
    let (l, r) = stereoize(&mono, Stereo::Mono, 44_100);
    assert_eq!(l, r);
    let (l, r) = stereoize(&mono, Stereo::Wide { amount: 0.8 }, 44_100);
    assert_ne!(l, r); // decorrelated channels differ...
    let mid_rms = rms(&l
        .iter()
        .zip(&r)
        .map(|(a, b)| (a + b) / 2.0)
        .collect::<Vec<_>>());
    assert!(mid_rms > 0.1); // ...but the mid (mono sum) survives
    let (l, r) = stereoize(&mono, Stereo::Haas { ms: 10.0, pan: 1.0 }, 44_100);
    let delay = (0.010 * 44_100.0) as usize;
    assert_eq!(l[delay..delay + 100], mono[..100]); // left trails by 10 ms
    assert_eq!(r[..100], mono[..100]); // right leads
}

#[test]
fn fm_seq_strikes_bright_then_mellows() {
    // One sustained fm note: the decaying modulation index makes the
    // attack brighter than the tail — the hammer-strike signature.
    let d = doc(r#"{ "name": "n", "duration": 1.0, "root": { "type": "seq",
                 "bpm": 60, "steps_per_beat": 1, "wave": "fm",
                 "fm_ratio": 1.0, "fm_index": 6, "fm_strike": 0.15,
                 "env": { "d": 0.9, "s": 0.5 },
                 "notes": [ { "step": 0, "len": 1, "pitch": "A3" } ] } }"#);
    let s = render(&d);
    assert!(rms(&s) > 0.05, "fm note audible");
    let third = s.len() / 3;
    assert!(
        brightness(&s[..third]) > brightness(&s[2 * third..]) * 1.5,
        "strike should be brighter than the tail"
    );
}

#[test]
fn pluck_seq_rings_and_decays_deterministically() {
    let json = r#"{ "name": "n", "duration": 1.2, "seed": 9, "root": { "type": "seq",
            "bpm": 60, "steps_per_beat": 1, "wave": "pluck", "pluck_decay": 0.995,
            "env": { "d": 0.1, "s": 1.0 },
            "notes": [ { "step": 0, "len": 1, "pitch": "A3" } ] } }"#;
    let s = render(&doc(json));
    let half = s.len() / 2;
    assert!(rms(&s[..half]) > 0.05, "pluck audible");
    assert!(
        rms(&s[half..]) < rms(&s[..half]) * 0.5,
        "string decays naturally"
    );
    // Same seed ⇒ identical string; different seed ⇒ different noise burst.
    assert_eq!(s, render(&doc(json)));
    let mut other = doc(json);
    other.seed = 10;
    assert_ne!(s, render(&other));
}

#[test]
fn piano_bass_rings_longer_than_treble() {
    let note = |pitch: &str| {
        let d = doc(&format!(
            r#"{{ "name": "n", "duration": 2.0, "root": {{ "type": "seq",
                     "bpm": 60, "steps_per_beat": 1, "wave": "piano",
                     "env": {{ "a": 0.002, "s": 1.0, "r": 0.1 }},
                     "notes": [ {{ "step": 0, "len": 2, "pitch": "{pitch}" }} ] }} }}"#
        ));
        render(&d)
    };
    let tail_ratio = |s: &[f32]| {
        let q = s.len() / 4;
        rms(&s[2 * q..3 * q]) / rms(&s[..q]).max(1e-9)
    };
    let bass = note("A1");
    let treble = note("A5");
    assert!(rms(&bass) > 0.02 && rms(&treble) > 0.005, "both audible");
    assert!(
        tail_ratio(&bass) > tail_ratio(&treble) * 1.5,
        "bass sustains, treble dies: {} vs {}",
        tail_ratio(&bass),
        tail_ratio(&treble)
    );
}

#[test]
fn piano_tone_knobs_default_to_the_concert_grand() {
    // Omitting the piano_* keys must render byte-identically to setting them
    // at their documented defaults — the byte-safe contract for the knobs.
    let bare = r#"{ "name":"n", "duration":1.0, "engine":5, "root": { "type":"seq",
            "bpm":60, "steps_per_beat":1, "wave":"piano", "env": { "a":0.002, "s":1.0, "r":0.1 },
            "notes": [ { "step":0, "len":1, "pitch":"C4" } ] } }"#;
    let defaults = r#"{ "name":"n", "duration":1.0, "engine":5, "root": { "type":"seq",
            "bpm":60, "steps_per_beat":1, "wave":"piano", "env": { "a":0.002, "s":1.0, "r":0.1 },
            "piano_hammer":1.0, "piano_strike":0.125, "piano_inharm":1.0, "piano_detune":1.0, "piano_decay":1.0,
            "notes": [ { "step":0, "len":1, "pitch":"C4" } ] } }"#;
    assert_eq!(
        render(&doc(bare)),
        render(&doc(defaults)),
        "the tone-knob defaults reproduce the grand bit-for-bit"
    );
}

#[test]
fn piano_variants_are_spectrally_distinct() {
    let piano = |extra: &str| {
        doc(&format!(
            r#"{{ "name":"n", "duration":1.5, "engine":5, "root": {{ "type":"seq",
                    "bpm":60, "steps_per_beat":1, "wave":"piano", "env": {{ "a":0.002, "s":1.0, "r":0.1 }},
                    {extra}
                    "notes": [ {{ "step":0, "len":1, "pitch":"C4", "gain":0.9 }} ] }} }}"#
        ))
    };
    let grand = render(&piano(""));
    let felt = render(&piano(
        r#""piano_hammer":0.35, "piano_strike":0.16, "piano_decay":0.8,"#,
    ));
    let honky = render(&piano(r#""piano_detune":12.0, "piano_inharm":1.7,"#));
    assert_ne!(grand, felt, "felt is a different waveform");
    assert_ne!(grand, honky, "honky-tonk is a different waveform");
    // Felt's soft hammer removes upper partials — less high-frequency energy
    // (a first-difference sum is a crude high-pass).
    let hf = |s: &[f32]| s.windows(2).map(|w| (w[1] - w[0]).abs()).sum::<f32>();
    assert!(
        hf(&felt) < hf(&grand),
        "felt is darker than the grand: {} vs {}",
        hf(&felt),
        hf(&grand)
    );
}

#[test]
fn kit_styles_are_distinct_bounded_and_default_to_classic() {
    let kit = |style: &str| {
        let key = if style.is_empty() {
            String::new()
        } else {
            format!(r#""kit":"{style}", "#)
        };
        doc(&format!(
            r#"{{ "name":"n", "duration":1.0, "engine":5, "root": {{ "type":"seq",
                    "bpm":120, "steps_per_beat":4, "wave":"kit", "env": {{ "a":0.001, "s":1.0, "r":0.05 }}, {key}
                    "notes": [ {{"step":0,"len":1,"pitch":"midi:36"}}, {{"step":2,"len":1,"pitch":"midi:38"}},
                               {{"step":4,"len":1,"pitch":"midi:42"}}, {{"step":6,"len":1,"pitch":"midi:49"}} ] }} }}"#
        ))
    };
    let peak = |s: &[f32]| s.iter().fold(0.0f32, |m, &x| m.max(x.abs()));
    let classic = render(&kit(""));
    let acoustic = render(&kit("acoustic"));
    let electronic = render(&kit("electronic"));
    let eight = render(&kit("808"));
    for (name, s) in [
        ("acoustic", &acoustic),
        ("electronic", &electronic),
        ("808", &eight),
    ] {
        assert!(s.iter().all(|x| x.is_finite()), "{name}: no NaN/inf");
        assert!(
            peak(s) > 0.05 && peak(s) < 2.5,
            "{name} audible+bounded: {}",
            peak(s)
        );
        assert_ne!(&classic, s, "{name} differs from the classic kit");
    }
    assert_ne!(acoustic, electronic, "acoustic and electronic differ");
    assert_ne!(electronic, eight, "electronic and 808 differ");
    // Omitting `kit` renders identically to selecting `classic`.
    assert_eq!(
        render(&kit("")),
        render(&kit("classic")),
        "default == classic"
    );
}

#[test]
fn bass_tone_knobs_default_to_the_current_voice_and_variants_differ() {
    let bass = |extra: &str| {
        doc(&format!(
            r#"{{ "name":"n", "duration":1.5, "engine":5, "root": {{ "type":"seq",
                    "bpm":90, "steps_per_beat":2, "wave":"bass", "env": {{ "a":0.005, "d":0.1, "s":0.9, "r":0.12 }},
                    {extra}
                    "notes": [ {{"step":0,"len":4,"pitch":"E1","gain":0.9}} ] }} }}"#
        ))
    };
    // Byte-safe: omitting the bass_* keys == setting them at their defaults.
    let bare = render(&bass(""));
    let defaults = render(&bass(
        r#""bass_cutoff":250.0,"bass_env":700.0,"bass_env_vel":1100.0,"bass_decay":0.15,"bass_click":0.0,"bass_body":0.7,"bass_sub":0.45,"bass_sub_ratio":1.0,"bass_drive":0.0,"bass_body_decay":2.0,"#,
    ));
    assert_eq!(bare, defaults, "bass defaults reproduce the current voice");
    // A driven synth-bass variant is a different, well-formed waveform.
    let synth = render(&bass(
        r#""bass_cutoff":600.0,"bass_drive":0.35,"bass_sub_ratio":0.5,"bass_body_decay":6.0,"#,
    ));
    assert!(synth.iter().all(|x| x.is_finite()));
    assert_ne!(bare, synth, "synth bass differs from finger");
    // The octave-down sub (ratio 0.5) puts real energy below the fundamental.
    let octave = render(&bass(r#""bass_sub":0.9,"bass_sub_ratio":0.5,"#));
    assert_ne!(bare, octave, "octave-down sub changes the voice");
}

#[test]
fn guitar_tone_stages_default_to_identity_and_variants_differ() {
    let pluck = |extra: &str| {
        doc(&format!(
            r#"{{ "name":"n", "duration":1.2, "engine":5, "seed":3, "root": {{ "type":"seq",
                    "bpm":90, "steps_per_beat":2, "wave":"pluck", "pluck_decay":0.96, "env": {{ "a":0.001, "s":1.0, "r":0.2 }},
                    {extra}
                    "notes": [ {{"step":0,"len":4,"pitch":"E3","gain":0.9}} ] }} }}"#
        ))
    };
    // Byte-safe: omitting the stages == setting them at their identity defaults.
    let bare = render(&pluck(""));
    let defaults = render(&pluck(
        r#""pluck_body":0.0,"pluck_pick":0.0,"pluck_tone":0.0,"#,
    ));
    assert_eq!(bare, defaults, "the pluck tone stages default to a no-op");
    // A bodied, dark nylon variant is a different, well-formed waveform.
    let nylon = render(&pluck(
        r#""pluck_body":0.55,"pluck_pick":0.05,"pluck_tone":-0.35,"#,
    ));
    assert!(nylon.iter().all(|x| x.is_finite()));
    assert_ne!(bare, nylon, "nylon body/tone/pick change the voice");
}

fn one_note(wave: &str, pitch: &str, secs: f32) -> Vec<f32> {
    let d = doc(&format!(
        r#"{{ "name": "n", "duration": {secs}, "root": {{ "type": "seq",
                 "bpm": 60, "steps_per_beat": 1, "wave": "{wave}",
                 "env": {{ "a": 0.002, "s": 1.0, "r": 0.05 }},
                 "notes": [ {{ "step": 0, "len": {len}, "pitch": "{pitch}" }} ] }} }}"#,
        len = secs.ceil() as u32,
    ));
    render(&d)
}

#[test]
fn epiano_tine_pings_then_mellows() {
    let s = one_note("epiano", "A3", 1.0);
    assert!(rms(&s) > 0.05, "epiano audible");
    let q = s.len() / 4;
    assert!(brightness(&s[..q]) > brightness(&s[3 * q..]) * 1.3);
}

#[test]
fn organ_sustains_while_held() {
    let s = one_note("organ", "C3", 1.0);
    assert!(rms(&s) > 0.1, "organ audible");
    let q = s.len() / 4;
    // No natural decay: the last quarter holds level with the second.
    let (mid, tail) = (rms(&s[q..2 * q]), rms(&s[3 * q..]));
    assert!(tail > mid * 0.7, "organ holds: {mid} -> {tail}");
}

#[test]
fn strings_swell_in_slowly() {
    let s = one_note("strings", "A3", 1.0);
    assert!(rms(&s) > 0.05, "strings audible");
    let ms50 = 44_100 / 20;
    // The bow swell: the first 50 ms is much quieter than the body.
    assert!(rms(&s[..ms50]) < rms(&s[ms50 * 6..ms50 * 8]) * 0.6);
}

#[test]
fn bass_is_darker_than_a_raw_saw() {
    let b = one_note("bass", "E2", 0.5);
    let saw = one_note("sawtooth", "E2", 0.5);
    assert!(rms(&b) > 0.05, "bass audible");
    assert!(
        brightness(&b) < brightness(&saw) * 0.5,
        "bass is filtered dark"
    );
}

#[test]
fn new_melodic_waves_are_audible_finite_and_deterministic() {
    for wave in ["brass", "flute", "mallet", "bell"] {
        let a = one_note(wave, "A3", 0.5);
        let b = one_note(wave, "A3", 0.5);
        assert!(rms(&a) > 0.02, "{wave} audible, rms {}", rms(&a));
        assert!(a.iter().all(|x| x.is_finite()), "{wave}: no NaN/inf");
        assert_eq!(a, b, "{wave} must be deterministic");
    }
}

#[test]
fn brass_filter_swells_open() {
    let s = one_note("brass", "C3", 0.8);
    let win = |a: f32, b: f32| brightness(&s[(a * 44_100.0) as usize..(b * 44_100.0) as usize]);
    // The blat: the tone is measurably brighter once the cutoff has opened
    // (compare just after the attack to the fully-open steady state, before
    // the detune beating confounds the proxy).
    assert!(
        win(0.15, 0.2) > win(0.01, 0.03) * 1.2,
        "brass brightens as the filter opens"
    );
}

#[test]
fn mallet_strike_dies_into_the_fundamental() {
    let s = one_note("mallet", "A3", 0.5);
    let q = s.len() / 4;
    assert!(brightness(&s[..q]) > brightness(&s[3 * q..]) * 1.3);
}

#[test]
fn bell_highs_die_before_the_hum() {
    let s = one_note("bell", "A3", 1.0);
    let q = s.len() / 4;
    assert!(brightness(&s[..q]) > brightness(&s[3 * q..]) * 1.3);
}

#[test]
fn tracks_pan_places_instruments_on_the_stage() {
    let d = doc(
        r#"{ "name": "n", "duration": 0.2, "root": { "type": "tracks", "tracks": [
                { "pan": -1.0, "node": { "type": "sine", "freq": 440 } },
                { "pan":  1.0, "gain": 0.5, "node": { "type": "sine", "freq": 660 } }
            ] } }"#,
    );
    assert_eq!(d.validate(), Ok(()));
    let tr = render_tracks(&d).unwrap();
    let (l, r) = (tr.left, tr.right);
    // Hard-left 440 dominates L; hard-right (at half gain) is alone on R.
    assert!(
        rms(&l) > rms(&r) * 1.5,
        "left louder: {} vs {}",
        rms(&l),
        rms(&r)
    );
    let zero_crossings = |s: &[f32]| s.windows(2).filter(|w| w[0] * w[1] < 0.0).count();
    // R carries only the 660 Hz track ⇒ more crossings per second.
    assert!(zero_crossings(&r) > zero_crossings(&l));
    // The public mono render is the mid of the same bus.
    let mid = render(&d);
    assert!((mid[1000] - 0.5 * (l[1000] + r[1000])).abs() < 1e-6);
}

#[test]
fn tracks_master_reverb_decorrelates_the_channels() {
    let d = doc(
        r#"{ "name": "n", "duration": 0.5, "root": { "type": "tracks",
                 "tracks": [ { "node": { "type": "mul", "inputs": [
                     { "type": "sine", "freq": 440 },
                     { "type": "env", "d": 0.1 } ] } } ],
                 "master": [ { "type": "reverb", "room": 0.6, "mix": 0.4 } ] } }"#,
    );
    let tr = render_tracks(&d).unwrap();
    let (l, r) = (tr.left, tr.right);
    assert_ne!(l, r, "spread reverb gives each side its own tail");
    // And with a duck in the master, both channels stay deterministic.
    let d2 = doc(
        r#"{ "name": "n", "duration": 0.5, "seed": 3, "root": { "type": "tracks",
                 "tracks": [ { "node": { "type": "noise" } } ],
                 "master": [ { "type": "duck", "amount": 0.7,
                   "trigger": { "type": "seq", "bpm": 120, "steps_per_beat": 1,
                     "wave": "kit", "env": { "s": 1 },
                     "notes": [ { "step": 0, "len": 1, "pitch": "midi:36" } ] } } ] } }"#,
    );
    let a = render_tracks(&d2).unwrap();
    let b = render_tracks(&d2).unwrap();
    assert_eq!(a, b, "stereo master bus renders are byte-stable");
}

#[test]
fn tracks_validation_guards_the_console() {
    let nested = doc(r#"{ "name": "n", "root": { "type": "mix", "inputs": [
                { "type": "tracks", "tracks": [ { "node": { "type": "noise" } } ] }
            ] } }"#);
    assert!(nested.validate().unwrap_err().contains("root"));
    let bad_master = doc(r#"{ "name": "n", "root": { "type": "tracks",
                 "tracks": [ { "node": { "type": "noise" } } ],
                 "master": [ { "type": "sine", "freq": 440 } ] } }"#);
    assert!(bad_master.validate().unwrap_err().contains("master"));
    let bad_pan = doc(r#"{ "name": "n", "root": { "type": "tracks",
                 "tracks": [ { "pan": 2.0, "node": { "type": "noise" } } ] } }"#);
    assert!(bad_pan.validate().unwrap_err().contains("pan"));
}

#[test]
fn sampler_requires_a_soundfont_path_and_exposes_it_to_loaders() {
    // validate() is filesystem-free (the core is pure compute): a nonexistent
    // path validates, and sf2_paths() hands it to the loader to check.
    let d = doc(r#"{ "name": "n", "duration": 0.5, "root": { "type": "seq",
                 "bpm": 120, "wave": "sampler", "sf2": "/no/such/font.sf2",
                 "env": { "s": 1 },
                 "notes": [ { "step": 0, "len": 2, "pitch": "C4" } ] } }"#);
    d.validate().expect("path existence is the loader's job");
    assert_eq!(d.sf2_paths(), vec!["/no/such/font.sf2"]);
    // An empty path is still a structural error.
    let d = doc(r#"{ "name": "n", "duration": 0.5, "root": { "type": "seq",
                 "bpm": 120, "wave": "sampler",
                 "env": { "s": 1 },
                 "notes": [ { "step": 0, "len": 2, "pitch": "C4" } ] } }"#);
    assert!(d.validate().unwrap_err().contains("sf2"));
}

/// Full sampler audio check — needs a real SoundFont. Set
/// TONO_TEST_SF2=/path/to/any_gm_bank.sf2 to enable; skipped (and
/// printed as such) otherwise so CI stays hermetic.
#[test]
fn sampler_renders_real_instruments_deterministically() {
    let Some(sf2) = std::env::var_os("TONO_TEST_SF2") else {
        eprintln!("skipping sampler audio test: TONO_TEST_SF2 not set");
        return;
    };
    let sf2 = sf2.to_string_lossy().replace('"', "");
    let d = doc(&format!(
        r#"{{ "name": "n", "duration": 2.0, "root": {{ "type": "seq",
                 "bpm": 120, "wave": "sampler", "sf2": "{sf2}", "sf2_preset": 0,
                 "env": {{ "s": 1 }},
                 "notes": [ {{ "step": 0, "len": 2, "pitch": "C4" }},
                            {{ "step": 2, "len": 2, "pitch": "E4" }},
                            {{ "step": 4, "len": 4, "pitch": "G4" }} ] }} }}"#
    ));
    let s = render(&d);
    assert!(rms(&s) > 0.01, "sampled piano audible");
    assert_eq!(s, render(&d), "sampler render is deterministic");
    // Percussion bank: a GM kick on channel 9.
    let k = doc(&format!(
        r#"{{ "name": "n", "duration": 1.0, "root": {{ "type": "seq",
                 "bpm": 120, "wave": "sampler", "sf2": "{sf2}", "sf2_bank": 128,
                 "env": {{ "s": 1 }},
                 "notes": [ {{ "step": 0, "len": 2, "pitch": "midi:36" }} ] }} }}"#
    ));
    assert!(rms(&render(&k)[..8820]) > 0.01, "sampled kick audible");
}

#[test]
fn duck_pumps_a_pad_under_its_trigger() {
    // A steady pad ducked by a kick pattern: rms right after each kick is
    // lower than between kicks.
    let d = doc(
        r#"{ "name": "n", "duration": 1.0, "root": { "type": "chain", "stages": [
                { "type": "sine", "freq": 220 },
                { "type": "duck", "amount": 0.9, "release": 0.2,
                  "trigger": { "type": "seq", "bpm": 120, "steps_per_beat": 1,
                    "wave": "kit", "env": { "s": 1 },
                    "notes": [ { "step": 0, "len": 1, "pitch": "midi:36" },
                               { "step": 1, "len": 1, "pitch": "midi:36" } ] } }
            ] } }"#,
    );
    let s = render(&d);
    let sr = 44_100;
    // 60 ms right after the kick at t=0 vs the recovered region ~0.4 s.
    let after_kick = rms(&s[..sr * 6 / 100]);
    let recovered = rms(&s[sr * 2 / 5..sr * 45 / 100]);
    assert!(
        after_kick < recovered * 0.65,
        "pumped {after_kick} vs recovered {recovered}"
    );
}

#[test]
fn swing_delays_offbeats_and_humanize_jitters_deterministically() {
    let beat = |extra: &str| {
        let d = doc(&format!(
            r#"{{ "name": "n", "duration": 1.0, "root": {{ "type": "seq",
                     "bpm": 120, "steps_per_beat": 2, "wave": "sine"{extra},
                     "env": {{ "d": 0.05 }},
                     "notes": [ {{ "step": 0, "len": 1, "pitch": 880 }},
                                {{ "step": 1, "len": 1, "pitch": 880 }} ] }} }}"#
        ));
        render(&d)
    };
    let onset =
        |s: &[f32], from: usize| from + s[from..].iter().position(|x| x.abs() > 0.05).unwrap();
    let straight = beat("");
    let swung = beat(r#", "swing": 0.6"#);
    // Step 1 (the off-beat, at 0.25 s) lands later when swung; step 0 doesn't.
    let half = 44_100 / 5; // search after 0.2 s
    assert_eq!(onset(&straight, 0), onset(&swung, 0));
    let (a, b) = (onset(&straight, half), onset(&swung, half));
    let expected = (0.6 * 0.5 * 0.25 * 44_100.0) as usize; // swing*half*step
    assert!(
        (b - a) as i64 - expected as i64 <= 2,
        "off-beat delayed by ~{expected}, got {}",
        b - a
    );
    // Humanize changes timing/level but is deterministic.
    let h1 = beat(r#", "humanize": 0.3"#);
    let h2 = beat(r#", "humanize": 0.3"#);
    assert_eq!(h1, h2);
    assert_ne!(h1, straight);
}

#[test]
fn cowbell_knocks_and_tracks_pitch() {
    let lo = one_note("cowbell", "A4", 1.0);
    let hi = one_note("cowbell", "A5", 1.0);
    assert!(rms(&lo[..4410]) > 0.1, "cowbell knocks");
    assert!(brightness(&hi) > brightness(&lo), "pitch tracks the note");
    // Fast knock decay: the tail is near-silent.
    assert!(rms(&lo[lo.len() / 2..]) < 0.01);
    // And the kit's fixed cowbell (GM 56) responds too.
    let kit = one_note("kit", "midi:56", 0.3);
    assert!(rms(&kit[..4410]) > 0.05, "kit cowbell audible");
}

#[test]
fn kit_maps_pitches_to_distinct_drums() {
    let kick = one_note("kit", "midi:36", 0.4);
    let snare = one_note("kit", "midi:38", 0.4);
    let hat = one_note("kit", "midi:42", 0.4);
    for (name, s) in [("kick", &kick), ("snare", &snare), ("hat", &hat)] {
        assert!(rms(s) > 0.01, "{name} audible");
    }
    // Spectral ordering: kick < snare < hat.
    assert!(brightness(&kick) < brightness(&snare));
    assert!(brightness(&snare) < brightness(&hat));
    // Hat dies fast; open hat (midi:46) rings longer.
    let open = one_note("kit", "midi:46", 0.4);
    let q = hat.len() / 4;
    assert!(rms(&open[q..2 * q]) > rms(&hat[q..2 * q]) * 2.0);
    // Noise-based drums stay deterministic.
    assert_eq!(snare, one_note("kit", "midi:38", 0.4));
}

#[test]
fn seq_with_absurd_note_lengths_stays_bounded() {
    // A 4-billion-step note and a near-zero bpm must not allocate
    // note-length buffers beyond the render window (OOM guard).
    let d = doc(r#"{ "name": "n", "duration": 0.1, "root": { "type": "seq",
                 "bpm": 120, "wave": "square", "env": { "d": 0.05 },
                 "notes": [ { "step": 0, "len": 4000000000, "pitch": 440 } ] } }"#);
    assert_eq!(render(&d).len(), 4410);
    let d = doc(r#"{ "name": "n", "duration": 0.1, "root": { "type": "seq",
                 "bpm": 0.0001, "wave": "sine", "env": { "d": 0.05 },
                 "notes": [ { "step": 0, "len": 1, "pitch": 440 } ] } }"#);
    assert_eq!(render(&d).len(), 4410);
}

#[test]
fn mix_layers_and_mul_gates() {
    let d = doc(
        r#"{ "name": "n", "duration": 0.05, "root": { "type": "mix", "inputs": [
                { "type": "sine", "freq": 220 },
                { "type": "sine", "freq": 330 }
            ] } }"#,
    );
    assert!(rms(&render(&d)) > 0.5);
    let d = doc(
        r#"{ "name": "n", "duration": 0.05, "root": { "type": "mul", "inputs": [
                { "type": "sine", "freq": 220 },
                { "type": "gain", "amount": 1 }
            ] } }"#,
    );
    // gain (a processor) standalone renders silence; mul with silence = silence.
    assert!(rms(&render(&d)) < 1e-6);
}

#[test]
fn unvalidated_docs_render_without_panicking() {
    // The codebase's contract: render() must never panic, even on documents
    // that skipped validate() — the kernels clamp defensively.
    let d = doc(r#"{ "name": "n", "duration": 0.05, "root": { "type": "seq",
                 "bpm": 120, "wave": "piano", "piano_inharm": -1, "env": { "d": 0.05 },
                 "notes": [ { "step": 0, "len": 1, "pitch": 220 } ] } }"#);
    assert!(d.validate().is_err(), "the doc is genuinely unvalidated");
    let s = render(&d);
    assert!(s.iter().all(|x| x.is_finite()));

    let d = doc(
        r#"{ "name": "n", "duration": 0.05, "root": { "type": "chain",
                 "stages": [ { "type": "noise" }, { "type": "bitcrush", "bits": 32 } ] } }"#,
    );
    assert!(d.validate().is_err());
    let s = render(&d);
    assert!(s.iter().all(|x| x.is_finite()));

    // An absurd sample rate inverts the pluck's clamp bounds (used to panic).
    let d = doc(r#"{ "name": "n", "duration": 0.05, "sample_rate": 20,
                 "root": { "type": "seq", "bpm": 120, "wave": "pluck", "env": { "d": 0.05 },
                 "notes": [ { "step": 0, "len": 1, "pitch": 220 } ] } }"#);
    assert!(d.validate().is_err());
    let s = render(&d);
    assert!(s.iter().all(|x| x.is_finite()));
}

#[test]
fn extreme_but_valid_docs_render_finite() {
    // Everything at (or just inside) a validation cap must render finite
    // audio — the caps exist to keep the kernels out of the overflow regime.
    let cases = [
        r#"{ "name": "n", "duration": 0.05, "root": { "type": "super", "freq": 110,
                 "detune_cents": 12000 } }"#,
        r#"{ "name": "n", "duration": 0.05, "root": { "type": "fm", "freq": 100000,
                 "ratio": 4096, "index": 1 } }"#,
        r#"{ "name": "n", "duration": 0.05, "root": { "type": "chain", "stages": [
                { "type": "noise" },
                { "type": "lowpass",
                  "cutoff": { "rand": { "from": 200, "to": 1200, "rate": 10000 } } }
            ] } }"#,
        r#"{ "name": "n", "duration": 0.05, "root": { "type": "chain", "stages": [
                { "type": "noise" }, { "type": "compress", "threshold": 0.01, "ratio": 1e30 }
            ] } }"#,
        r#"{ "name": "n", "duration": 0.05, "root": { "type": "sine", "freq": "midi:160" } }"#,
    ];
    for json in cases {
        let d = doc(json);
        d.validate().unwrap_or_else(|e| panic!("{json}: {e}"));
        let s = render(&d);
        assert!(s.iter().all(|x| x.is_finite()), "{json}: non-finite sample");
    }
}

#[test]
fn peak_limit_scrubs_non_finite_samples() {
    // A NaN/inf buffer used to measure peak 0 (f32::max discards NaN) and
    // flow un-limited to the encoders; now it is scrubbed to silence first.
    let mut ch = [0.5f32, f32::NAN, f32::INFINITY, -0.25];
    crate::dsp::peak_limit(&mut [&mut ch]);
    assert!(ch.iter().all(|x| x.is_finite()));
    assert_eq!(ch[1], 0.0);
    assert_eq!(ch[3], -0.25);
}

#[test]
fn loudness_gated_tolerates_mismatched_channel_lengths() {
    let a = [0.5f32; 4410];
    let b = [0.5f32; 2205];
    let l = crate::dsp::loudness_lufs_gated(&[&a, &b], 44_100);
    assert!(l.is_finite());
}

#[cfg(feature = "analysis")]
#[test]
fn wavetable_position_morphs_the_spectrum() {
    let at = |pos: &str| {
        doc(&format!(
            r#"{{ "name": "w", "duration": 0.5, "root": {{ "type": "wavetable",
                     "wave": "basic", "freq": 220, "position": {pos} }} }}"#
        ))
    };
    let centroid = |d: SoundDoc| {
        let s = render_graph(&d);
        crate::analysis::stats(&s, 44_100).spectral_centroid_hz
    };
    let dark = centroid(at("0.0")); // the sine frame
    let bright = centroid(at("1.0")); // the saw frame
    assert!(
        dark < bright,
        "basic at 0.0 ({dark} Hz) should be darker than at 1.0 ({bright} Hz)"
    );
}

#[test]
fn wavetable_is_deterministic_and_finite_at_extremes() {
    // Determinism: the same doc (modulated morph, note-name freq) renders
    // bit-equal twice.
    let d = doc(r#"{ "name": "w", "duration": 0.1, "seed": 3, "root":
             { "type": "wavetable", "wave": "formant", "freq": "A3",
               "position": { "lfo": { "shape": "sine", "rate": 5, "depth": 0.5, "center": 0.5 } } } }"#);
    let a = render_graph(&d);
    let b = render_graph(&d);
    let bits = |s: &[f32]| s.iter().map(|x| x.to_bits()).collect::<Vec<_>>();
    assert_eq!(bits(&a), bits(&b), "same doc renders bit-identically");

    // Extreme frequencies and positions stay finite on every table.
    for wave in ["basic", "harmonics", "formant", "metallic"] {
        for (freq, pos) in [("1.0", "0.0"), ("20000.0", "1.0"), ("100000.0", "0.5")] {
            let d = doc(&format!(
                r#"{{ "name": "w", "duration": 0.02, "root": {{ "type": "wavetable",
                         "wave": "{wave}", "freq": {freq}, "position": {pos} }} }}"#
            ));
            let s = render_graph(&d);
            assert!(
                s.iter().all(|x| x.is_finite()),
                "{wave} at {freq} Hz / position {pos}: non-finite sample"
            );
        }
    }
}

/// Tracks-level sidechain: the pad (hard right, so the right channel is the
/// pad alone) follows the kick (hard left). Right after each kick hit the
/// pad's level is pulled down versus the same window in an unducked render —
/// the pump is measurable on the bus.
#[test]
fn tracks_sidechain_pumps_the_follower() {
    // 120 bpm, 1 step/beat ⇒ a hit at 0.0 s and 0.5 s.
    let mk = |sidechain: &str| {
        doc(&format!(
            r#"{{ "name": "t", "duration": 1.0, "seed": 5, "version": 2,
                    "root": {{ "type": "tracks", "tracks": [
                        {{ "id": "kick", "pan": -1.0,
                          "node": {{ "type": "seq", "bpm": 120, "steps_per_beat": 1, "wave": "sine",
                            "env": {{ "a": 0.001, "d": 0.12, "s": 0.0, "r": 0.02 }},
                            "notes": [ {{ "step": 0, "len": 1, "pitch": "A1" }},
                                       {{ "step": 1, "len": 1, "pitch": "A1" }} ] }} }},
                        {{ "id": "pad", "pan": 1.0, "gain": 0.5, {sidechain}
                          "node": {{ "type": "sine", "freq": 220 }} }}
                    ] }} }}"#
        ))
    };
    let ducked = render_tracks(&mk(
        r#""sidechain": { "source": "kick", "amount": 0.9, "release": 0.08 },"#,
    ))
    .unwrap();
    let open = render_tracks(&mk("")).unwrap();
    let sr = 44_100.0;
    let win = |s: &[f32], from: f32, to: f32| rms(&s[(from * sr) as usize..(to * sr) as usize]);
    for hit in [0.0f32, 0.5] {
        let (d, o) = (
            win(&ducked.right, hit + 0.01, hit + 0.05),
            win(&open.right, hit + 0.01, hit + 0.05),
        );
        assert!(
            d < o * 0.4,
            "the ~50 ms after the hit at {hit}: ducked rms {d} vs open rms {o}"
        );
    }
    // Mid-gap the envelope has released and the pad is back at full level.
    let (d, o) = (win(&ducked.right, 0.7, 0.8), win(&open.right, 0.7, 0.8));
    assert!(d > o * 0.8, "mid-gap the duck is open: rms {d} vs open {o}");
    // The source renders untouched: its measured bus contribution (peak and
    // RMS stats) is identical whether or not a follower is attached.
    assert_eq!(ducked.layers[0].peak_dbfs, open.layers[0].peak_dbfs);
    assert_eq!(ducked.layers[0].rms_dbfs, open.layers[0].rms_dbfs);
}

/// amount 0 means no ducking: the render is bit-identical to the same
/// document without the sidechain field. (The follower is declared FIRST, so
/// this also proves render order does not depend on declaration order.)
#[test]
fn tracks_sidechain_amount_zero_is_byte_identical() {
    let with = doc(r#"{ "name": "t", "duration": 0.3, "seed": 2, "version": 2,
            "root": { "type": "tracks", "tracks": [
                { "id": "pad", "node": { "type": "sine", "freq": 220 }, "gain": 0.5,
                  "sidechain": { "source": "kick", "amount": 0.0 } },
                { "id": "kick", "node": { "type": "noise" }, "gain": 0.5 }
            ] } }"#);
    let without = doc(r#"{ "name": "t", "duration": 0.3, "seed": 2, "version": 2,
            "root": { "type": "tracks", "tracks": [
                { "id": "pad", "node": { "type": "sine", "freq": 220 }, "gain": 0.5 },
                { "id": "kick", "node": { "type": "noise" }, "gain": 0.5 }
            ] } }"#);
    let (a, b) = (
        render_tracks(&with).unwrap(),
        render_tracks(&without).unwrap(),
    );
    let bits = |s: &[f32]| s.iter().map(|x| x.to_bits()).collect::<Vec<_>>();
    assert_eq!(bits(&a.left), bits(&b.left), "left byte-identical");
    assert_eq!(bits(&a.right), bits(&b.right), "right byte-identical");
}

/// The duck envelope is computed causally from the source's rendered signal —
/// no randomness — so a sidechained document renders bit-identically twice.
#[test]
fn tracks_sidechain_is_deterministic() {
    let d = doc(r#"{ "name": "t", "duration": 0.3, "seed": 11, "version": 2,
            "root": { "type": "tracks", "tracks": [
                { "id": "kick", "node": { "type": "noise" }, "gain": 0.5 },
                { "id": "pad", "node": { "type": "sine", "freq": 220 }, "gain": 0.5,
                  "sidechain": { "source": "kick", "amount": 0.8 } }
            ] } }"#);
    let a = render_tracks(&d).unwrap();
    let b = render_tracks(&d).unwrap();
    assert_eq!(a, b, "two renders are bit-identical");
}

// --- convolve (offline-only FFT convolution reverb) ---

/// A click through convolve rings: the bare impact is dead within a few ms,
/// but the convolved tail is clearly audible well past 50 ms and decays.
#[test]
fn convolve_adds_a_decaying_tail_to_a_click() {
    let wet = doc(r#"{ "name": "c", "duration": 1.0, "seed": 9,
            "root": { "type": "chain", "stages": [
                { "type": "impact", "hardness": 0.9, "velocity": 1.0 },
                { "type": "convolve", "decay": 0.6, "mix": 1.0 }
            ] } }"#);
    let dry = doc(r#"{ "name": "c", "duration": 1.0, "seed": 9,
            "root": { "type": "impact", "hardness": 0.9, "velocity": 1.0 } }"#);
    let (w, d) = (render_graph(&wet), render_graph(&dry));
    let sr = 44_100usize;
    // The tail window: well after the strike, well inside the decay.
    let (a, b) = ((0.05 * sr as f32) as usize, (0.4 * sr as f32) as usize);
    let (tail_w, tail_d) = (rms(&w[a..b]), rms(&d[a..b]));
    assert!(
        tail_d < 1e-4,
        "bare impact is silent after 50 ms, got {tail_d}"
    );
    assert!(
        tail_w > 10.0 * tail_d && tail_w > 1e-4,
        "convolve tail rings: wet {tail_w} vs dry {tail_d}"
    );
    // And the tail decays: the later window is quieter than the earlier one.
    let late = rms(&w[(0.5 * sr as f32) as usize..(0.9 * sr as f32) as usize]);
    assert!(late < tail_w, "tail decays: early {tail_w}, late {late}");
}

/// mix 0 is a transparent passthrough: bit-identical to the same chain
/// without the convolve stage.
#[test]
fn convolve_mix_zero_is_bit_identical_passthrough() {
    let with = doc(r#"{ "name": "c", "duration": 0.3, "seed": 4,
            "root": { "type": "chain", "stages": [
                { "type": "impact", "hardness": 0.7 },
                { "type": "convolve", "decay": 1.0, "damp": 0.6, "mix": 0.0 }
            ] } }"#);
    let without = doc(r#"{ "name": "c", "duration": 0.3, "seed": 4,
            "root": { "type": "impact", "hardness": 0.7 } }"#);
    let bits = |s: &[f32]| s.iter().map(|x| x.to_bits()).collect::<Vec<_>>();
    assert_eq!(
        bits(&render_graph(&with)),
        bits(&render_graph(&without)),
        "mix 0 passes the input through untouched"
    );
}

/// Two renders of the same document are bit-identical — the synthesized IR is
/// stable per graph position (drawn from the node's structural seed).
#[test]
fn convolve_is_deterministic_per_graph_position() {
    let d = doc(r#"{ "name": "c", "duration": 0.5, "seed": 7,
            "root": { "type": "chain", "stages": [
                { "type": "noise", "color": "pink" },
                { "type": "convolve", "decay": 0.4, "predelay": 0.02, "mix": 0.5 }
            ] } }"#);
    let bits = |s: &[f32]| s.iter().map(|x| x.to_bits()).collect::<Vec<_>>();
    assert_eq!(bits(&render_graph(&d)), bits(&render_graph(&d)));
}

/// The IR is seeded from the document's seed (via the structural path), so a
/// different doc seed gives a different room; the same seed always the same.
#[test]
fn convolve_ir_tracks_the_doc_seed() {
    let mk = |seed: u64| {
        doc(&format!(
            r#"{{ "name": "c", "duration": 0.5, "seed": {seed},
                "root": {{ "type": "chain", "stages": [
                    {{ "type": "impact", "hardness": 0.8 }},
                    {{ "type": "convolve", "decay": 0.5, "mix": 1.0 }}
                ] }} }}"#
        ))
    };
    let (a, b) = (render_graph(&mk(1)), render_graph(&mk(2)));
    assert_ne!(a, b, "a different seed builds a different IR");
}

/// The wet tail folds into the document length (like reverb) — the render
/// never extends past the document, and stays finite at the extremes.
#[test]
fn convolve_output_is_bounded_and_finite() {
    let d = doc(r#"{ "name": "c", "duration": 0.2, "seed": 3,
            "root": { "type": "chain", "stages": [
                { "type": "noise" },
                { "type": "convolve", "decay": 30.0, "predelay": 0.05, "damp": 1.0, "mix": 0.9 }
            ] } }"#);
    let out = render_graph(&d);
    assert_eq!(out.len(), (0.2f32 * 44_100.0).ceil() as usize);
    assert!(out.iter().all(|x| x.is_finite()));
}

// --- granular (offline-only granular texture) ---

/// mix 0 is a transparent passthrough, bit-identical to the bare source.
#[test]
fn granular_mix_zero_is_bit_identical_passthrough() {
    let with = doc(r#"{ "name": "g", "duration": 0.3, "seed": 5,
            "root": { "type": "chain", "stages": [
                { "type": "sine", "freq": 220 },
                { "type": "granular", "grain_ms": 60, "density": 40, "pitch": 2.0,
                  "spread": 0.7, "mix": 0.0 }
            ] } }"#);
    let without = doc(r#"{ "name": "g", "duration": 0.3, "seed": 5,
            "root": { "type": "sine", "freq": 220 } }"#);
    let bits = |s: &[f32]| s.iter().map(|x| x.to_bits()).collect::<Vec<_>>();
    assert_eq!(bits(&render_graph(&with)), bits(&render_graph(&without)));
}

/// The grain schedule is drawn up front from the node's structural seed, in a
/// fixed order — two renders of the same doc are bit-identical, and a doc
/// with no randomization (spread 0) still renders sanely at minimal density.
#[test]
fn granular_is_deterministic_and_sane_at_low_density() {
    let d = doc(r#"{ "name": "g", "duration": 1.0, "seed": 8,
            "root": { "type": "chain", "stages": [
                { "type": "sine", "freq": 330 },
                { "type": "granular", "grain_ms": 100, "density": 0.1, "mix": 0.5 }
            ] } }"#);
    let bits = |s: &[f32]| s.iter().map(|x| x.to_bits()).collect::<Vec<_>>();
    let (a, b) = (render_graph(&d), render_graph(&d));
    assert_eq!(bits(&a), bits(&b), "two renders are bit-identical");
    assert!(a.iter().all(|x| x.is_finite()));
    // Density 0.1: barely any grains land in the window, so the output is
    // dominated by the dry half of the crossfade — never silent, never loud.
    let level = rms(&a);
    assert!(level > 1e-3 && level < 2.0, "sane level {level}");
}

/// Every knob at its validation extreme still renders finite audio.
#[test]
fn granular_stays_finite_at_extreme_params() {
    let d = doc(r#"{ "name": "g", "duration": 0.5, "seed": 12,
            "root": { "type": "chain", "stages": [
                { "type": "noise", "color": "white" },
                { "type": "granular", "grain_ms": 500, "density": 200, "pitch": 4.0,
                  "spread": 1.0, "mix": 1.0 }
            ] } }"#);
    let out = render_graph(&d);
    assert!(out.iter().all(|x| x.is_finite()));
    let d = doc(r#"{ "name": "g", "duration": 0.5, "seed": 12,
            "root": { "type": "chain", "stages": [
                { "type": "noise", "color": "white" },
                { "type": "granular", "grain_ms": 5, "density": 0.1, "pitch": 0.25,
                  "spread": 0.0, "mix": 1.0 }
            ] } }"#);
    assert!(render_graph(&d).iter().all(|x| x.is_finite()));
}

/// A pitch-2 granular texture moves the energy up an octave: the peak of the
/// magnitude spectrum moves from the source's 220 Hz to ~440 Hz.
#[test]
fn granular_pitch_two_shifts_the_spectrum_up() {
    let mk = |pitch: f32| {
        doc(&format!(
            r#"{{ "name": "g", "duration": 1.0, "seed": 6,
                "root": {{ "type": "chain", "stages": [
                    {{ "type": "sine", "freq": 220 }},
                    {{ "type": "granular", "grain_ms": 80, "density": 50,
                        "pitch": {pitch}, "spread": 0.0, "mix": 1.0 }}
                ] }} }}"#
        ))
    };
    let peak_hz = |s: &[f32]| {
        // Middle of the buffer (skip the overlap-faded edges).
        let n = 8192usize;
        let start = 44_100 / 2 - n / 2;
        let mut planner = rustfft::FftPlanner::<f32>::new();
        let fft = planner.plan_fft_forward(n);
        let mut buf: Vec<rustfft::num_complex::Complex<f32>> = s[start..start + n]
            .iter()
            .map(|&x| rustfft::num_complex::Complex::new(x, 0.0))
            .collect();
        fft.process(&mut buf);
        let peak = buf[..n / 2]
            .iter()
            .enumerate()
            .max_by(|a, b| a.1.norm().total_cmp(&b.1.norm()))
            .map(|(k, _)| k)
            .unwrap();
        peak as f32 * 44_100.0 / n as f32
    };
    let one = peak_hz(&render_graph(&mk(1.0)));
    let two = peak_hz(&render_graph(&mk(2.0)));
    assert!(
        (one - 220.0).abs() < 20.0,
        "pitch 1 stays at the source: {one} Hz"
    );
    assert!(
        (two - 440.0).abs() < 40.0,
        "pitch 2 shifts the peak up an octave: {two} Hz"
    );
}

/// A track routed to a bus reaches the master through the bus's return:
/// a zero-gain bus silences the routed track while a direct track plays.
#[test]
fn tracks_bus_routes_through_the_return_fader() {
    let mk = |bus_gain: f32| {
        doc(&format!(
            r#"{{ "name":"t", "duration":0.3, "seed":1, "version":2,
                "root":{{ "type":"tracks", "buses":[ {{ "id":"drums", "gain":{bus_gain} }} ],
                  "tracks":[
                    {{ "id":"routed", "node":{{ "type":"sawtooth", "freq":220 }}, "gain":1.0, "bus":"drums" }},
                    {{ "id":"direct", "node":{{ "type":"sine", "freq":440 }}, "gain":0.1 }}
                  ] }} }}"#
        ))
    };
    let open = render_tracks(&mk(1.0)).unwrap();
    let shut = render_tracks(&mk(0.0)).unwrap();
    // The direct track sounds in both; the routed track vanishes with the
    // return fader down.
    let body = |r: &TracksRender| rms(&r.left[..r.left.len() / 4]);
    assert!(
        body(&shut) < body(&open) * 0.3,
        "the return fader gates the routed track: {} vs {}",
        body(&open),
        body(&shut)
    );
}

/// A post-fader send feeds a copy into the bus: a short note sent to a
/// reverb bus leaves a tail past its own length; the same mix without the
/// send doesn't.
#[test]
fn tracks_send_feeds_a_reverb_bus() {
    let mk = |send: &str| {
        doc(&format!(
            r#"{{ "name":"t", "duration":1.0, "seed":1, "version":2,
                "root":{{ "type":"tracks",
                  "buses":[ {{ "id":"verb", "gain":1.0, "effects":[ {{ "type":"reverb", "room":0.7, "mix":1.0 }} ] }} ],
                  "tracks":[
                    {{ "id":"hit", "node":{{ "type":"mul", "inputs":[
                        {{ "type":"sawtooth", "freq":220 }},
                        {{ "type":"env", "a":0.001, "d":0.1, "s":0.0, "r":0.02 }} ] }},
                      "sends":[ {{ "bus":"verb", "amount":{send} }} ] }}
                  ] }} }}"#
        ))
    };
    let sent = render_tracks(&mk("0.8")).unwrap();
    let dry = render_tracks(&mk("0.0")).unwrap();
    // The note dies by ~0.15 s; the tail past 0.5 s is only there via the send.
    let n = sent.left.len();
    let tail = |r: &TracksRender| rms(&r.left[n / 2..]);
    assert!(
        tail(&sent) > tail(&dry) * 3.0 + 1e-6,
        "the send leaves a reverb tail"
    );
    assert!(
        rms(&sent.left[..n / 8]) > 1e-3,
        "the dry note still sounds up front"
    );
}

/// Bus inserts process only what reaches the bus: a lowpass on the bus
/// darkens the routed track but leaves the direct track untouched.
#[test]
fn bus_inserts_process_only_the_routed_signal() {
    let d = doc(r#"{ "name":"t", "duration":0.3, "seed":1, "version":2,
        "root":{ "type":"tracks",
          "buses":[ { "id":"dark", "gain":1.0, "effects":[ { "type":"lowpass", "cutoff":300, "q":0.7 } ] } ],
          "tracks":[
            { "id":"routed", "node":{ "type":"sawtooth", "freq":220 }, "bus":"dark" },
            { "id":"direct", "node":{ "type":"sawtooth", "freq":220 } }
          ] } }"#);
    let r = render_tracks(&d).unwrap();
    let bright = |s: &[f32]| {
        // Cheap brightness: high-frequency energy via first differences.
        s.windows(2).map(|w| (w[1] - w[0]).abs()).sum::<f32>()
    };
    // The same saw direct vs filtered: compare against a no-insert mix.
    let plain = doc(r#"{ "name":"t", "duration":0.3, "seed":1, "version":2,
        "root":{ "type":"tracks",
          "buses":[ { "id":"dark", "gain":1.0 } ],
          "tracks":[
            { "id":"routed", "node":{ "type":"sawtooth", "freq":220 }, "bus":"dark" },
            { "id":"direct", "node":{ "type":"sawtooth", "freq":220 } }
          ] } }"#);
    let p = render_tracks(&plain).unwrap();
    assert!(
        bright(&r.left) < bright(&p.left) * 0.9,
        "the bus insert darkens the mix: {} vs {}",
        bright(&r.left),
        bright(&p.left)
    );
}

/// The new fields round-trip through serde (and stay out of the way when
/// absent — older documents parse unchanged).
#[test]
fn buses_and_sends_round_trip_through_serde() {
    let d = doc(r#"{ "name":"t", "duration":0.1, "seed":1, "version":2,
        "root":{ "type":"tracks",
          "buses":[ { "id":"verb", "gain":0.8, "effects":[ { "type":"reverb", "room":0.5, "mix":0.3 } ] } ],
          "tracks":[
            { "id":"a", "node":{ "type":"sine", "freq":440 }, "bus":"verb",
              "sends":[ { "bus":"verb", "amount":0.4 } ] }
          ] } }"#);
    d.validate().expect("a wired-up bus mix validates");
    let json = serde_json::to_string(&d).unwrap();
    let back: SoundDoc = serde_json::from_str(&json).unwrap();
    let bits = |s: &[f32]| s.iter().map(|x| x.to_bits()).collect::<Vec<_>>();
    let a = render_tracks(&d).unwrap();
    let b = render_tracks(&back).unwrap();
    assert_eq!(
        bits(&a.left),
        bits(&b.left),
        "serde round-trip keeps the render"
    );
}

/// Stems decompose the mix: every track (muted ones silent) plus every bus
/// return, and their sum is the pre-master mix exactly.
#[test]
fn stems_decompose_the_pre_master_mix() {
    let d = doc(
        r#"{ "name":"t", "duration":0.5, "seed":2, "version":2, "engine":5,
        "root":{ "type":"tracks",
          "buses":[ { "id":"verb", "gain":0.8, "effects":[ { "type":"reverb", "room":0.5, "mix":0.5 } ] } ],
          "tracks":[
            { "id":"kick", "node":{ "type":"sine", "freq":80 }, "gain":0.2,
              "sends":[ { "bus":"verb", "amount":0.4 } ] },
            { "id":"pad", "node":{ "type":"sawtooth", "freq":110 }, "gain":0.15, "bus":"verb" },
            { "id":"mute", "node":{ "type":"sine", "freq":220 }, "mute":true }
          ],
          "master":[ { "type":"reverb", "room":0.2, "mix":0.2 } ] } }"#,
    );
    let stems = render_stems(&d).unwrap();
    assert_eq!(stems.len(), 4, "three tracks + one bus");
    assert_eq!(stems[0].id, "kick");
    assert!(!stems[0].is_bus);
    assert_eq!(stems[1].id, "pad");
    assert_eq!(stems[2].id, "mute");
    assert!(
        stems[2].left.iter().all(|&x| x == 0.0),
        "muted stem is silent"
    );
    assert_eq!(stems[3].id, "bus:verb");
    assert!(stems[3].is_bus);
    // kick is the only direct-to-master track; pad reaches the master only
    // through the bus. Sum the stems and compare to the mix pre-master:
    // recomputing the mix without its master chain is the reference.
    let plain = doc(
        r#"{ "name":"t", "duration":0.5, "seed":2, "version":2, "engine":5,
        "root":{ "type":"tracks",
          "buses":[ { "id":"verb", "gain":0.8, "effects":[ { "type":"reverb", "room":0.5, "mix":0.5 } ] } ],
          "tracks":[
            { "id":"kick", "node":{ "type":"sine", "freq":80 }, "gain":0.2,
              "sends":[ { "bus":"verb", "amount":0.4 } ] },
            { "id":"pad", "node":{ "type":"sawtooth", "freq":110 }, "gain":0.15, "bus":"verb" },
            { "id":"mute", "node":{ "type":"sine", "freq":220 }, "mute":true }
          ] } }"#,
    );
    let reference = render_tracks(&plain).unwrap();
    let n = reference.left.len();
    let (mut sl, mut sr) = (vec![0.0f32; n], vec![0.0f32; n]);
    for s in stems.iter().filter(|s| s.is_bus || s.bus.is_none()) {
        for i in 0..n {
            sl[i] += s.left[i];
            sr[i] += s.right[i];
        }
    }
    // Direct-to-master track stems plus the bus returns reproduce the mix
    // the master chain hears (the pad's raw stem is its channel output for
    // external processing — it's already inside bus:verb, so it's excluded).
    // The limiter is transparent for this quiet mix.
    let head = n / 2;
    let diff: f32 = (0..head)
        .map(|i| (sl[i] - reference.left[i]).abs())
        .fold(0.0, f32::max);
    assert!(
        diff < 1e-6,
        "stems sum to the pre-master mix (max diff {diff})"
    );
}
