use super::*;
use crate::dsl::SoundDoc;
use crate::dsp::Rng;
use crate::render::render_graph;

fn bits(s: &[f32]) -> Vec<u32> {
    s.iter().map(|x| x.to_bits()).collect()
}

fn parse(json: &str) -> SoundDoc {
    serde_json::from_str(json).unwrap()
}

/// Assert a doc streams byte-for-byte identical to the offline graph, in one
/// block and split across several block sizes.
fn assert_byte_identical(doc: &SoundDoc) {
    let offline = render_graph(doc);
    let mut sg = StreamGraph::try_from_doc(doc).expect("should be streamable");
    let mut whole = vec![0.0f32; offline.len()];
    sg.fill(&mut whole);
    assert_eq!(
        bits(&whole),
        bits(&offline),
        "whole-block stream != offline"
    );
    for bs in [1usize, 7, 64, 333] {
        let mut sg = StreamGraph::try_from_doc(doc).unwrap();
        let mut got: Vec<f32> = Vec::with_capacity(offline.len());
        while got.len() < offline.len() {
            let take = bs.min(offline.len() - got.len());
            let mut blk = vec![0.0f32; take];
            sg.fill(&mut blk);
            got.extend(blk);
        }
        assert_eq!(bits(&got), bits(&offline), "block size {bs} != offline");
    }
}

#[test]
fn filtered_square() {
    assert_byte_identical(&parse(
        r#"{ "name":"s", "duration":0.1, "root": { "type":"chain", "stages": [
            { "type":"square", "freq":220 },
            { "type":"lowpass", "cutoff":800, "q":0.7 } ] } }"#,
    ));
}

#[test]
fn set_pitch_transposes_byte_identically() {
    // Live pitch is a true repitch: a 220 Hz oscillator at pitch ×2 is
    // bit-for-bit a 660 Hz oscillator — same phase increment every sample.
    let mut lo = StreamGraph::try_from_doc(&parse(
        r#"{ "name":"a", "duration":0.05, "root": { "type":"sawtooth", "freq":220 } }"#,
    ))
    .unwrap();
    lo.set_pitch(3.0);
    let mut hi = StreamGraph::try_from_doc(&parse(
        r#"{ "name":"a", "duration":0.05, "root": { "type":"sawtooth", "freq":660 } }"#,
    ))
    .unwrap();
    let (mut a, mut b) = (vec![0.0f32; 1024], vec![0.0f32; 1024]);
    lo.fill(&mut a);
    hi.fill(&mut b);
    assert_eq!(bits(&a), bits(&b), "pitch ×3 on 220 Hz == 660 Hz");
}

#[test]
fn set_cutoff_sweeps_the_filter_and_is_identity_at_one() {
    let doc = parse(
        r#"{ "name":"s", "duration":0.05, "root": { "type":"chain", "stages": [
            { "type":"sawtooth", "freq":220 },
            { "type":"lowpass", "cutoff":4000, "q":0.7 } ] } }"#,
    );
    let rms = |s: &[f32]| (s.iter().map(|x| x * x).sum::<f32>() / s.len() as f32).sqrt();

    // scale 1.0 recomputes to the exact baked coefficients — byte-identical.
    let mut base = StreamGraph::try_from_doc(&doc).unwrap();
    let mut same = StreamGraph::try_from_doc(&doc).unwrap();
    same.set_cutoff(1.0);
    let (mut a, mut b) = (vec![0.0f32; 1024], vec![0.0f32; 1024]);
    base.fill(&mut a);
    same.fill(&mut b);
    assert_eq!(bits(&a), bits(&b), "cutoff scale 1.0 is identity");

    // Closing the lowpass (scale down) strips the saw's upper harmonics.
    let mut dark = StreamGraph::try_from_doc(&doc).unwrap();
    dark.set_cutoff(0.15); // 4000 Hz → ~600 Hz
    let mut d = vec![0.0f32; 1024];
    dark.fill(&mut d);
    assert!(rms(&d) < rms(&a), "closing the lowpass darkens the tone");
}

#[test]
fn glide_eases_pitch_toward_target_without_jumping() {
    let mut g = StreamGraph::try_from_doc(&parse(
        r#"{ "name":"a", "duration":1.0, "root": { "type":"sine", "freq":220 } }"#,
    ))
    .unwrap();
    g.glide_pitch(2.0, 0.0005); // slow portamento up an octave
    let mut one = vec![0.0f32; 1];
    g.fill(&mut one);
    assert!(g.pitch() < 1.05, "does not jump on the first sample");
    let mut long = vec![0.0f32; 40_000];
    g.fill(&mut long);
    assert!(g.pitch() > 1.9, "eases most of the way to the target");
    assert!(g.pitch() <= 2.0, "never overshoots the target");
}

#[test]
fn mix_of_oscillators() {
    assert_byte_identical(&parse(
        r#"{ "name":"m", "duration":0.05, "root": { "type":"mix", "inputs": [
            { "type":"sine", "freq":440 },
            { "type":"sawtooth", "freq":110 } ] } }"#,
    ));
}

#[test]
fn lfo_modulated_frequency() {
    assert_byte_identical(&parse(
        r#"{ "name":"l", "duration":0.08, "root":
            { "type":"sine", "freq": { "lfo": { "shape":"sine", "rate":6, "depth":80, "center":440 } } } }"#,
    ));
}

#[test]
fn slide_and_arp_modulators() {
    assert_byte_identical(&parse(
        r#"{ "name":"sl", "duration":0.1, "root":
            { "type":"sawtooth", "freq": { "slide": { "from":110, "to":880, "secs":0.09, "curve":"lin" } } } }"#,
    ));
    assert_byte_identical(&parse(
        r#"{ "name":"ar", "duration":0.1, "root":
            { "type":"square", "freq": { "arp": { "steps":[220,330,440], "rate":20 } } } }"#,
    ));
}

#[test]
fn rand_modulator_carries_its_walk() {
    assert_byte_identical(&parse(
        r#"{ "name":"rn", "duration":0.1, "root":
            { "type":"sine", "freq": { "rand": { "from":200, "to":600, "rate":15, "seed":42 } } } }"#,
    ));
}

#[test]
fn fm_and_super_sources() {
    assert_byte_identical(&parse(
        r#"{ "name":"fm", "duration":0.05, "root": { "type":"fm", "freq":220, "ratio":2.0, "index":5.0 } }"#,
    ));
    assert_byte_identical(&parse(
        r#"{ "name":"su", "duration":0.05, "root":
            { "type":"super", "wave":"sawtooth", "freq":110, "voices":7, "detune_cents":18 } }"#,
    ));
}

#[test]
fn impact_and_env() {
    assert_byte_identical(&parse(
        r#"{ "name":"im", "duration":0.05, "root": { "type":"impact", "hardness":0.6, "velocity":0.9 } }"#,
    ));
    assert_byte_identical(&parse(
        r#"{ "name":"ev", "duration":0.2, "root": { "type":"mul", "inputs": [
            { "type":"sine", "freq":330 },
            { "type":"env", "adsr": { "a":0.01, "d":0.05, "s":0.4, "r":0.1 } } ] } }"#,
    ));
}

#[test]
fn peak_and_shelf_eq() {
    assert_byte_identical(&parse(
        r#"{ "name":"eq", "duration":0.06, "root": { "type":"chain", "stages": [
            { "type":"sawtooth", "freq":150 },
            { "type":"peak", "cutoff":1200, "q":1.5, "gain_db":6 },
            { "type":"lowshelf", "cutoff":200, "gain_db":-4 },
            { "type":"highshelf", "cutoff":4000, "gain_db":3 } ] } }"#,
    ));
}

#[test]
fn delay_reverb_and_modal_effects() {
    assert_byte_identical(&parse(
        r#"{ "name":"dl", "duration":0.15, "root": { "type":"chain", "stages": [
            { "type":"sawtooth", "freq":110 },
            { "type":"delay", "secs":0.03, "feedback":0.4 } ] } }"#,
    ));
    assert_byte_identical(&parse(
        r#"{ "name":"rv", "duration":0.1, "root": { "type":"chain", "stages": [
            { "type":"impact", "hardness":0.7, "velocity":0.9 },
            { "type":"reverb", "room":0.8, "mix":0.5 } ] } }"#,
    ));
    assert_byte_identical(&parse(
        r#"{ "name":"md", "duration":0.1, "root": { "type":"chain", "stages": [
            { "type":"impact", "hardness":0.9, "velocity":1.0 },
            { "type":"modal", "modes": [
                { "freq":300, "decay":0.4, "gain":1.0 },
                { "freq":740, "decay":0.25, "gain":0.6 } ], "mix":0.8 } ] } }"#,
    ));
}

#[test]
fn modulation_effects_chorus_flanger_phaser() {
    for eff in [
        r#"{ "type":"chorus", "rate":1.5, "depth":0.6, "mix":0.5 }"#,
        r#"{ "type":"flanger", "rate":0.8, "depth":0.7, "feedback":0.5, "mix":0.6 }"#,
        r#"{ "type":"phaser", "rate":0.5, "depth":0.8, "feedback":0.4, "mix":0.7 }"#,
    ] {
        assert_byte_identical(&parse(&format!(
            r#"{{ "name":"fx", "duration":0.12, "root": {{ "type":"chain", "stages": [
                {{ "type":"sawtooth", "freq":220 }}, {eff} ] }} }}"#
        )));
    }
}

#[test]
fn dynamics_and_waveshaping() {
    assert_byte_identical(&parse(
        r#"{ "name":"cp", "duration":0.1, "root": { "type":"chain", "stages": [
            { "type":"square", "freq":150 },
            { "type":"compress", "threshold":-18, "ratio":4, "attack":0.005, "release":0.08, "makeup":3 } ] } }"#,
    ));
    assert_byte_identical(&parse(
        r#"{ "name":"dv", "duration":0.06, "engine":1, "root": { "type":"chain", "stages": [
            { "type":"sine", "freq":200 },
            { "type":"drive", "amount":6, "shape":"tanh" } ] } }"#,
    ));
    assert_byte_identical(&parse(
        r#"{ "name":"bc", "duration":0.06, "root": { "type":"chain", "stages": [
            { "type":"sawtooth", "freq":180 },
            { "type":"bitcrush", "bits":5 },
            { "type":"downsample", "factor":4 },
            { "type":"ringmod", "freq":300 } ] } }"#,
    ));
}

#[test]
fn duck_with_streamable_trigger() {
    assert_byte_identical(&parse(
        r#"{ "name":"dk", "duration":0.12, "root": { "type":"chain", "stages": [
            { "type":"sawtooth", "freq":110 },
            { "type":"duck", "amount":0.8, "attack":0.005, "release":0.05,
              "trigger": { "type":"square", "freq":4 } } ] } }"#,
    ));
}

// ---- randomized byte-identity fuzz over the streamable node set ----

fn rf(rng: &mut Rng, lo: f64, hi: f64) -> f64 {
    lo + (hi - lo) * (rng.next_u64() >> 11) as f64 / (1u64 << 53) as f64
}

fn gen_freq(rng: &mut Rng) -> serde_json::Value {
    use serde_json::json;
    if rng.next_u64().is_multiple_of(4) {
        json!({ "lfo": { "shape": "sine", "rate": rf(rng, 1.0, 8.0), "depth": rf(rng, 10.0, 120.0), "center": rf(rng, 200.0, 800.0) } })
    } else {
        json!(rf(rng, 80.0, 1200.0))
    }
}

fn gen_proc(rng: &mut Rng) -> serde_json::Value {
    use serde_json::json;
    let cut = rf(rng, 200.0, 4000.0);
    match rng.next_u64() % 11 {
        0 => json!({ "type":"lowpass", "cutoff":cut, "q":rf(rng,0.4,2.0) }),
        1 => json!({ "type":"highpass", "cutoff":cut, "q":rf(rng,0.4,2.0) }),
        2 => json!({ "type":"bandpass", "cutoff":cut, "q":rf(rng,0.4,2.0) }),
        3 => {
            json!({ "type":"peak", "cutoff":cut, "q":rf(rng,0.5,3.0), "gain_db":rf(rng,-8.0,8.0) })
        }
        4 => json!({ "type":"gain", "amount":rf(rng,0.3,1.2) }),
        5 => json!({ "type":"delay", "secs":rf(rng,0.005,0.04), "feedback":rf(rng,0.0,0.6) }),
        6 => json!({ "type":"reverb", "room":rf(rng,0.2,0.9), "mix":rf(rng,0.2,0.7) }),
        7 => json!({ "type":"drive", "amount":rf(rng,1.0,8.0), "shape":"tanh" }),
        8 => {
            json!({ "type":"chorus", "rate":rf(rng,0.5,3.0), "depth":rf(rng,0.3,0.9), "mix":rf(rng,0.3,0.7) })
        }
        9 => json!({ "type":"bitcrush", "bits": 3 + (rng.next_u64()%8) as u32 }),
        _ => {
            json!({ "type":"compress", "threshold":rf(rng,-24.0,-6.0), "ratio":rf(rng,2.0,8.0), "attack":0.005, "release":0.06, "makeup":rf(rng,0.0,4.0) })
        }
    }
}

fn gen_src(rng: &mut Rng, depth: u32) -> serde_json::Value {
    use serde_json::json;
    let leaf = depth == 0;
    let pick = rng.next_u64() % if leaf { 6 } else { 9 };
    match pick {
        0 => json!({ "type":"sine", "freq": gen_freq(rng) }),
        1 => json!({ "type":"square", "freq": gen_freq(rng), "duty": rf(rng, 0.2, 0.8) }),
        2 => json!({ "type":"sawtooth", "freq": gen_freq(rng) }),
        3 => json!({ "type":"triangle", "freq": gen_freq(rng) }),
        4 => {
            json!({ "type":"fm", "freq": gen_freq(rng), "ratio": rf(rng,1.0,4.0), "index": gen_freq(rng) })
        }
        5 => {
            json!({ "type":"super", "wave":"sawtooth", "freq": gen_freq(rng), "voices": 2 + (rng.next_u64()%6) as u32, "detune_cents": rf(rng,4.0,30.0) })
        }
        6 => json!({ "type":"mix", "inputs": [gen_src(rng, depth-1), gen_src(rng, depth-1)] }),
        7 => json!({ "type":"mul", "inputs": [gen_src(rng, depth-1),
                { "type":"env", "adsr": { "a":rf(rng,0.001,0.02), "d":rf(rng,0.02,0.1), "s":rf(rng,0.2,0.8), "r":rf(rng,0.05,0.2) } }] }),
        _ => {
            let mut stages = vec![gen_src(rng, depth - 1)];
            for _ in 0..(1 + rng.next_u64() % 3) {
                stages.push(gen_proc(rng));
            }
            json!({ "type":"chain", "stages": stages })
        }
    }
}

#[test]
fn fuzz_streamed_matches_offline_byte_for_byte() {
    use serde_json::json;
    let mut checked = 0;
    for seed in 0..250u64 {
        let mut rng = Rng::new(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ 0xABCD);
        let root = gen_src(&mut rng, 3);
        let dur = rf(&mut rng, 0.02, 0.08);
        let doc_json =
            json!({ "name":"fuzz", "duration": dur, "seed": seed, "engine": 1, "root": root });
        let Ok(doc) = serde_json::from_value::<SoundDoc>(doc_json) else {
            continue;
        };
        if doc.validate().is_err() || StreamGraph::try_from_doc(&doc).is_none() {
            continue;
        }
        assert_byte_identical(&doc);
        checked += 1;
    }
    assert!(
        checked > 120,
        "fuzz should exercise many graphs, got {checked}"
    );
}

#[test]
fn engine2_rng_leaves_stream_byte_identically() {
    for doc in [
        r#"{ "name":"nz", "duration":0.05, "seed":7, "engine":2, "root": { "type":"noise", "color":"pink" } }"#,
        r#"{ "name":"dz", "duration":0.08, "seed":9, "engine":2, "root": { "type":"dust", "density":800, "decay":0.02 } }"#,
        r#"{ "name":"wn", "duration":0.06, "seed":3, "engine":2, "root": { "type":"chain", "stages": [
            { "type":"noise", "color":"white" }, { "type":"lowpass", "cutoff":1200, "q":0.7 } ] } }"#,
        // Two noise siblings under a mix — proves order-independence (the whole
        // point of structural seeding): offline draws them contiguously, the
        // streamer per-sample-interleaved, yet the bytes match.
        r#"{ "name":"mn", "duration":0.05, "seed":5, "engine":2, "root": { "type":"mix", "inputs": [
            { "type":"noise", "color":"brown" }, { "type":"noise", "color":"white" } ] } }"#,
    ] {
        assert_byte_identical(&parse(doc));
    }
}

#[test]
fn engine2_seq_streams_byte_identically() {
    // A melodic square seq (no RNG voice).
    assert_byte_identical(&parse(
        r#"{ "name":"sq", "duration":0.4, "seed":11, "engine":2, "root": { "type":"seq",
            "bpm":120, "steps_per_beat":4, "wave":"square",
            "env": { "a":0.005, "d":0.05, "s":0.4, "r":0.08 },
            "notes": [ { "step":0, "len":2, "pitch":"C4" }, { "step":2, "len":2, "pitch":"E4" },
                       { "step":4, "len":2, "pitch":"G4" }, { "step":6, "len":2, "pitch":"C5" } ] } }"#,
    ));
    // A kit (noise-based drums) seq into reverb — the RNG-heavy path, streamed
    // through a stateful effect.
    assert_byte_identical(&parse(
        r#"{ "name":"dr", "duration":0.5, "seed":3, "engine":2, "root": { "type":"chain", "stages": [
            { "type":"seq", "bpm":140, "steps_per_beat":4, "wave":"kit",
              "env": { "a":0.001, "d":0.1, "s":0.0, "r":0.05 },
              "notes": [ { "step":0, "len":1, "pitch":"midi:36" }, { "step":2, "len":1, "pitch":"midi:38" },
                         { "step":4, "len":1, "pitch":"midi:42" }, { "step":6, "len":1, "pitch":"midi:38" } ] },
            { "type":"reverb", "room":0.5, "mix":0.3 } ] } }"#,
    ));
}

#[test]
fn engine3_piano_streams_byte_identically() {
    // The engine-3 inharmonic piano (RNG only for the hammer thump) must
    // pre-render and stream bit-for-bit, across the register.
    assert_byte_identical(&parse(
        r#"{ "name":"pno", "duration":1.2, "seed":8, "engine":3, "root": { "type":"seq",
            "bpm":90, "steps_per_beat":4, "wave":"piano",
            "env": { "a":0.002, "s":1.0, "r":0.2 },
            "notes": [ { "step":0, "len":4, "pitch":"A1" }, { "step":2, "len":4, "pitch":"C4" },
                       { "step":4, "len":4, "pitch":"E4", "gain":0.6 }, { "step":6, "len":4, "pitch":"A5" } ] } }"#,
    ));
}

#[test]
fn engine3_piano_variant_streams_byte_identically() {
    // A honky-tonk variant (wide detune, inharmonic, hard hammer) must still
    // pre-render and stream bit-for-bit.
    assert_byte_identical(&parse(
        r#"{ "name":"honk", "duration":1.0, "seed":4, "engine":3, "root": { "type":"seq",
            "bpm":90, "steps_per_beat":4, "wave":"piano",
            "piano_detune":12.0, "piano_inharm":1.7, "piano_hammer":1.5, "piano_strike":0.11, "piano_decay":0.65,
            "env": { "a":0.002, "s":1.0, "r":0.2 },
            "notes": [ { "step":0, "len":4, "pitch":"A3" }, { "step":4, "len":4, "pitch":"C4" } ] } }"#,
    ));
}

#[test]
fn kit_styles_stream_byte_identically() {
    // Each alternate kit keeps the one-draw-per-sample rng discipline, so the
    // pre-rendered stream matches the offline bounce bit-for-bit.
    for style in ["acoustic", "electronic", "808"] {
        assert_byte_identical(&parse(&format!(
            r#"{{ "name":"k", "duration":0.8, "seed":6, "engine":3, "root": {{ "type":"seq",
                "bpm":120, "steps_per_beat":4, "wave":"kit", "kit":"{style}", "env": {{ "a":0.001, "s":1.0, "r":0.05 }},
                "notes": [ {{"step":0,"len":1,"pitch":"midi:36"}}, {{"step":2,"len":1,"pitch":"midi:38"}},
                           {{"step":3,"len":1,"pitch":"midi:42"}}, {{"step":4,"len":1,"pitch":"midi:49"}},
                           {{"step":6,"len":1,"pitch":"midi:46"}} ] }} }}"#
        )));
    }
}

#[test]
fn bass_variant_streams_byte_identically() {
    // The bass voice draws no RNG, so every variant pre-renders and streams
    // bit-for-bit.
    assert_byte_identical(&parse(
        r#"{ "name":"b", "duration":1.0, "seed":2, "engine":3, "root": { "type":"seq",
            "bpm":100, "steps_per_beat":4, "wave":"bass",
            "bass_cutoff":600.0, "bass_env":1500.0, "bass_drive":0.35, "bass_sub_ratio":0.5, "bass_body_decay":6.0,
            "env": { "a":0.003, "d":0.06, "s":0.8, "r":0.08 },
            "notes": [ { "step":0, "len":4, "pitch":"E1" }, { "step":4, "len":4, "pitch":"G1" } ] } }"#,
    ));
}

#[test]
fn guitar_variant_streams_byte_identically() {
    // The pluck voice draws RNG (the KS burst); the new tone stages draw
    // none, so the draw order is unchanged and the nylon variant streams
    // bit-for-bit.
    assert_byte_identical(&parse(
        r#"{ "name":"g", "duration":1.0, "seed":5, "engine":3, "root": { "type":"seq",
            "bpm":100, "steps_per_beat":4, "wave":"pluck", "pluck_decay":0.9,
            "pluck_body":0.55, "pluck_pick":0.05, "pluck_tone":-0.35,
            "env": { "a":0.001, "s":1.0, "r":0.2 },
            "notes": [ { "step":0, "len":4, "pitch":"E3" }, { "step":4, "len":4, "pitch":"A3" } ] } }"#,
    ));
}

#[test]
fn engine1_noise_falls_back_but_engine2_streams() {
    // engine < 2 keeps the shared stream ⇒ not streamable (buffer fallback).
    assert!(
        StreamGraph::try_from_doc(&parse(
            r#"{ "name":"n1", "duration":0.05, "engine":1, "root": { "type":"noise", "color":"white" } }"#
        ))
        .is_none()
    );
    assert!(
        StreamGraph::try_from_doc(&parse(
            r#"{ "name":"n2", "duration":0.05, "engine":2, "root": { "type":"noise", "color":"white" } }"#
        ))
        .is_some()
    );
}

#[test]
fn non_streamable_graphs_are_rejected() {
    assert!(
        StreamGraph::try_from_doc(&parse(
            r#"{ "name":"n", "duration":0.05, "root": { "type":"noise", "color":"white" } }"#
        ))
        .is_none()
    );
    assert!(
        StreamGraph::try_from_doc(&parse(
            r#"{ "name":"t", "duration":0.05, "root": { "type":"tracks", "tracks": [
                { "node": { "type":"sine", "freq":440 } } ] } }"#
        ))
        .is_none()
    );
}

#[test]
fn loop_and_stereo_docs_fall_back_to_the_player() {
    // The streaming path has no loop-body or stereoize transform: playing
    // the raw graph would be un-looped / un-widened and not byte-identical
    // to the bounce.
    assert!(
        StreamGraph::try_from_doc(&parse(
            r#"{ "name":"l", "duration":0.5,
                "playback": { "mode":"loop", "start_secs":0.1, "crossfade_secs":0.05 },
                "root": { "type":"sine", "freq":220 } }"#
        ))
        .is_none()
    );
    assert!(
        StreamGraph::try_from_doc(&parse(
            r#"{ "name":"s", "duration":0.1,
                "stereo": { "mode":"haas", "ms":12 },
                "root": { "type":"sine", "freq":220 } }"#
        ))
        .is_none()
    );
}

#[test]
fn glide_pitch_nan_coeff_snaps_instead_of_poisoning() {
    // clamp() passes NaN through; a NaN glide coefficient used to latch the
    // pitch to NaN forever. It folds to an instant snap now.
    let d = parse(r#"{ "name":"s", "duration":0.1, "root": { "type":"sine", "freq": 440 } }"#);
    let mut g = StreamGraph::try_from_doc(&d).unwrap();
    g.glide_pitch(2.0, f32::NAN);
    let mut out = [0.0f32; 128];
    g.fill(&mut out);
    assert!(out.iter().all(|x| x.is_finite()));
    assert_eq!(g.pitch(), 2.0, "NaN coeff folds to an instant snap");
}
