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
pub(crate) fn assert_byte_identical(doc: &SoundDoc) {
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

/// Every node family, modulator and seq voice in one plain-root document
/// each: the corpus the block-size sweep runs over. A new node type gets a
/// line here.
const NODE_CORPUS: &[&str] = &[
    // Sources and the modulators on their parameters.
    r#"{ "name":"m", "duration":0.05, "root": { "type":"mix", "inputs": [
        { "type":"sine", "freq":440 }, { "type":"sawtooth", "freq":110 } ] } }"#,
    r#"{ "name":"l", "duration":0.08, "root":
        { "type":"sine", "freq": { "lfo": { "shape":"sine", "rate":6, "depth":80, "center":440 } } } }"#,
    r#"{ "name":"sl", "duration":0.1, "root":
        { "type":"sawtooth", "freq": { "slide": { "from":110, "to":880, "secs":0.09, "curve":"lin" } } } }"#,
    r#"{ "name":"ar", "duration":0.1, "root":
        { "type":"square", "freq": { "arp": { "steps":[220,330,440], "rate":20 } } } }"#,
    r#"{ "name":"rn", "duration":0.1, "root":
        { "type":"sine", "freq": { "rand": { "from":200, "to":600, "rate":15, "seed":42 } } } }"#,
    r#"{ "name":"fm", "duration":0.05, "root": { "type":"fm", "freq":220, "ratio":2.0, "index":5.0 } }"#,
    r#"{ "name":"su", "duration":0.05, "root":
        { "type":"super", "wave":"sawtooth", "freq":110, "voices":7, "detune_cents":18 } }"#,
    r#"{ "name":"im", "duration":0.05, "root": { "type":"impact", "hardness":0.6, "velocity":0.9 } }"#,
    r#"{ "name":"ev", "duration":0.2, "root": { "type":"mul", "inputs": [
        { "type":"sine", "freq":330 },
        { "type":"env", "adsr": { "a":0.01, "d":0.05, "s":0.4, "r":0.1 } } ] } }"#,
    // Structurally seeded leaves: offline draws each contiguously, the
    // stream per-sample-interleaved, and the bytes must still match.
    r#"{ "name":"nz", "duration":0.05, "seed":7, "engine":5, "root": { "type":"noise", "color":"pink" } }"#,
    r#"{ "name":"dz", "duration":0.08, "seed":9, "engine":5, "root": { "type":"dust", "density":800, "decay":0.02 } }"#,
    r#"{ "name":"mn", "duration":0.05, "seed":5, "engine":5, "root": { "type":"mix", "inputs": [
        { "type":"noise", "color":"brown" }, { "type":"noise", "color":"white" } ] } }"#,
    // Processors, stateful and closed-form, constant and modulated.
    r#"{ "name":"s", "duration":0.1, "root": { "type":"chain", "stages": [
        { "type":"square", "freq":220 }, { "type":"lowpass", "cutoff":800, "q":0.7 } ] } }"#,
    r#"{ "name":"wn", "duration":0.06, "seed":3, "engine":5, "root": { "type":"chain", "stages": [
        { "type":"noise", "color":"white" }, { "type":"lowpass", "cutoff":1200, "q":0.7 } ] } }"#,
    r#"{ "name":"eq", "duration":0.06, "root": { "type":"chain", "stages": [
        { "type":"sawtooth", "freq":150 },
        { "type":"peak", "cutoff":1200, "q":1.5, "gain_db":6 },
        { "type":"lowshelf", "cutoff":200, "gain_db":-4 },
        { "type":"highshelf", "cutoff":4000, "gain_db":3 } ] } }"#,
    r#"{ "name":"lp", "duration":0.1, "root": { "type":"chain", "stages": [
        { "type":"sawtooth", "freq":110 },
        { "type":"lowpass", "cutoff": { "lfo": { "rate":2, "depth":400, "center":800 } }, "q": 2.0 } ] } }"#,
    r#"{ "name":"pk", "duration":0.1, "root": { "type":"chain", "stages": [
        { "type":"noise" },
        { "type":"peak", "cutoff": { "slide": { "from": 300, "to": 3000, "secs": 0.08, "curve": "exp" } }, "gain_db": 9 },
        { "type":"highshelf", "cutoff": { "env": { "a": 0.01, "d": 0.05, "s": 0.2, "r": 0.02, "from": 2000, "to": 6000 } }, "gain_db": -6 } ] } }"#,
    r#"{ "name":"g", "duration":0.1, "root": { "type":"chain", "stages": [
        { "type":"sine", "freq":440 },
        { "type":"gain", "amount": { "env": { "a": 0.005, "d": 0.03, "s": 0.4, "r": 0.02, "from": 0.0, "to": 0.9 } } } ] } }"#,
    r#"{ "name":"dl", "duration":0.15, "root": { "type":"chain", "stages": [
        { "type":"sawtooth", "freq":110 }, { "type":"delay", "secs":0.03, "feedback":0.4 } ] } }"#,
    r#"{ "name":"rv", "duration":0.1, "root": { "type":"chain", "stages": [
        { "type":"impact", "hardness":0.7, "velocity":0.9 }, { "type":"reverb", "room":0.8, "mix":0.5 } ] } }"#,
    r#"{ "name":"md", "duration":0.1, "root": { "type":"chain", "stages": [
        { "type":"impact", "hardness":0.9, "velocity":1.0 },
        { "type":"modal", "modes": [
            { "freq":300, "decay":0.4, "gain":1.0 }, { "freq":740, "decay":0.25, "gain":0.6 } ], "mix":0.8 } ] } }"#,
    r#"{ "name":"ch", "duration":0.12, "root": { "type":"chain", "stages": [
        { "type":"sawtooth", "freq":220 }, { "type":"chorus", "rate":1.5, "depth":0.6, "mix":0.5 } ] } }"#,
    r#"{ "name":"tr", "duration":0.08, "root": { "type":"chain", "stages": [
        { "type":"sine", "freq":220 }, { "type":"tremolo", "rate":6, "depth":0.8 } ] } }"#,
    r#"{ "name":"cp", "duration":0.1, "root": { "type":"chain", "stages": [
        { "type":"square", "freq":150 },
        { "type":"compress", "threshold":-18, "ratio":4, "attack":0.005, "release":0.08, "makeup":3 } ] } }"#,
    r#"{ "name":"dv", "duration":0.06, "engine":5, "root": { "type":"chain", "stages": [
        { "type":"sine", "freq":200 }, { "type":"drive", "amount":6, "shape":"tanh" } ] } }"#,
    // The libm-heavy chains through the det kernels.
    r#"{ "name":"e5fx", "duration":0.3, "seed":3, "version":2, "engine":5,
        "root": { "type":"chain", "stages": [
            { "type":"sawtooth", "freq": { "slide": { "from":110, "to":220, "secs":0.25, "curve":"exp" } } },
            { "type":"lowpass", "cutoff":1400, "q":0.9 },
            { "type":"drive", "amount":4, "shape":"tanh" },
            { "type":"chorus", "rate":1.2, "depth":0.6, "mix":0.4 },
            { "type":"compress", "threshold":-16, "ratio":3, "attack":0.004, "release":0.07, "makeup":2 } ] } }"#,
    r#"{ "name":"e5tr", "duration":0.2, "seed":2, "version":2, "engine":5,
        "root": { "type":"chain", "stages": [
            { "type":"fm", "freq":330, "ratio":2.0, "index":2.5 },
            { "type":"tremolo", "rate":5.5, "depth":0.7 },
            { "type":"chorus", "rate":0.7, "depth":0.6, "mix":0.5 } ] } }"#,
    // Seq voices: pre-rendered at build and read back, the RNG-drawing ones
    // (kit, pluck, piano thump, flute breath) in sample order.
    r#"{ "name":"sq", "duration":0.4, "seed":11, "engine":5, "root": { "type":"seq",
        "bpm":120, "steps_per_beat":4, "wave":"square",
        "env": { "a":0.005, "d":0.05, "s":0.4, "r":0.08 },
        "notes": [ { "step":0, "len":2, "pitch":"C4" }, { "step":2, "len":2, "pitch":"E4" },
                   { "step":4, "len":2, "pitch":"G4" }, { "step":6, "len":2, "pitch":"C5" } ] } }"#,
    r#"{ "name":"dr", "duration":0.5, "seed":3, "engine":5, "root": { "type":"chain", "stages": [
        { "type":"seq", "bpm":140, "steps_per_beat":4, "wave":"kit",
          "env": { "a":0.001, "d":0.1, "s":0.0, "r":0.05 },
          "notes": [ { "step":0, "len":1, "pitch":"midi:36" }, { "step":2, "len":1, "pitch":"midi:38" },
                     { "step":4, "len":1, "pitch":"midi:42" }, { "step":6, "len":1, "pitch":"midi:38" } ] },
        { "type":"reverb", "room":0.5, "mix":0.3 } ] } }"#,
    r#"{ "name":"pno", "duration":1.2, "seed":8, "engine":5, "root": { "type":"seq",
        "bpm":90, "steps_per_beat":4, "wave":"piano",
        "env": { "a":0.002, "s":1.0, "r":0.2 },
        "notes": [ { "step":0, "len":4, "pitch":"A1" }, { "step":2, "len":4, "pitch":"C4" },
                   { "step":4, "len":4, "pitch":"E4", "gain":0.6 }, { "step":6, "len":4, "pitch":"A5" } ] } }"#,
    r#"{ "name":"honk", "duration":1.0, "seed":4, "engine":5, "root": { "type":"seq",
        "bpm":90, "steps_per_beat":4, "wave":"piano",
        "piano_detune":12.0, "piano_inharm":1.7, "piano_hammer":1.5, "piano_strike":0.11, "piano_decay":0.65,
        "env": { "a":0.002, "s":1.0, "r":0.2 },
        "notes": [ { "step":0, "len":4, "pitch":"A3" }, { "step":4, "len":4, "pitch":"C4" } ] } }"#,
    r#"{ "name":"b", "duration":1.0, "seed":2, "engine":5, "root": { "type":"seq",
        "bpm":100, "steps_per_beat":4, "wave":"bass",
        "bass_cutoff":600.0, "bass_env":1500.0, "bass_drive":0.35, "bass_sub_ratio":0.5, "bass_body_decay":6.0,
        "env": { "a":0.003, "d":0.06, "s":0.8, "r":0.08 },
        "notes": [ { "step":0, "len":4, "pitch":"E1" }, { "step":4, "len":4, "pitch":"G1" } ] } }"#,
    r#"{ "name":"gt", "duration":1.0, "seed":5, "engine":5, "root": { "type":"seq",
        "bpm":100, "steps_per_beat":4, "wave":"pluck", "pluck_decay":0.9,
        "pluck_body":0.55, "pluck_pick":0.05, "pluck_tone":-0.35,
        "env": { "a":0.001, "s":1.0, "r":0.2 },
        "notes": [ { "step":0, "len":4, "pitch":"E3" }, { "step":4, "len":4, "pitch":"A3" } ] } }"#,
    r#"{ "name":"e5fm", "duration":0.6, "seed":7, "version":2, "engine":5,
        "root": { "type":"seq", "bpm":132, "steps_per_beat":4, "wave":"fm",
          "fm_ratio":3.01, "fm_index":4.5, "fm_strike":0.09,
          "env": { "a":0.002, "d":0.06, "s":0.5, "r":0.08 },
          "notes": [ { "step":0, "len":2, "pitch":"C4" }, { "step":2, "len":2, "pitch":"Eb4", "gain":0.8 },
                     { "step":4, "len":2, "pitch":"G4" }, { "step":6, "len":2, "pitch":"midi:71", "gain":0.7 } ] } }"#,
    r#"{ "name":"e5kit", "duration":0.6, "seed":5, "version":2, "engine":5,
        "root": { "type":"seq", "bpm":120, "steps_per_beat":4, "wave":"kit", "kit":"808",
          "env": { "a":0.001, "d":0.1, "s":0.4, "r":0.06 },
          "notes": [ { "step":0, "len":1, "pitch":"midi:36" }, { "step":2, "len":1, "pitch":"midi:38", "gain":0.9 },
                     { "step":4, "len":1, "pitch":"midi:42", "gain":0.6 }, { "step":6, "len":1, "pitch":"midi:49", "gain":0.8 } ] } }"#,
];

#[test]
fn every_node_streams_byte_identically() {
    for json in NODE_CORPUS {
        let doc = parse(json);
        assert!(StreamGraph::blockers(&doc).is_empty(), "{json}");
        assert_byte_identical(&doc);
    }
    // The kit voicings and the fixed-model melodic voices, parametrized.
    for style in ["acoustic", "electronic", "808"] {
        assert_byte_identical(&parse(&format!(
            r#"{{ "name":"k", "duration":0.8, "seed":6, "engine":5, "root": {{ "type":"seq",
                "bpm":120, "steps_per_beat":4, "wave":"kit", "kit":"{style}", "env": {{ "a":0.001, "s":1.0, "r":0.05 }},
                "notes": [ {{"step":0,"len":1,"pitch":"midi:36"}}, {{"step":2,"len":1,"pitch":"midi:38"}},
                           {{"step":3,"len":1,"pitch":"midi:42"}}, {{"step":4,"len":1,"pitch":"midi:49"}},
                           {{"step":6,"len":1,"pitch":"midi:46"}} ] }} }}"#
        )));
    }
    for wave in [
        "brass", "flute", "mallet", "bell", "epiano", "organ", "strings", "cowbell",
    ] {
        assert_byte_identical(&parse(&format!(
            r#"{{ "name":"w", "duration":0.2, "seed":5, "engine":5, "root": {{ "type":"seq",
                "bpm":120, "steps_per_beat":4, "wave":"{wave}",
                "env": {{ "a":0.005, "d":0.05, "s":0.6, "r":0.05 }},
                "notes": [ {{ "step":0, "len":2, "pitch":"C4" }},
                           {{ "step":2, "len":2, "pitch":"G4", "gain":0.6 }} ] }} }}"#
        )));
    }
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

// ---- tracks root (mixer) ----

/// Assert a `tracks` doc streams byte-for-byte identical to the
/// offline mixer — the stereo bus, peak-limit gain included — in one block
/// and split across several block sizes. The gain is the runtime's
/// StreamSource probe mechanism: one throwaway pass of the same graph
/// measures the joint peak the offline output stage limited against.
fn assert_tracks_byte_identical(doc: &SoundDoc) {
    let product = crate::render::render_product(doc);
    let (el, er) = product.stereo.expect("a tracks doc renders stereo");
    let n = el.len();
    let gain = {
        let mut probe = StreamGraph::try_from_doc(doc).expect("should stream");
        let (mut l, mut r) = (vec![0.0f32; n], vec![0.0f32; n]);
        probe.fill_stereo(&mut l, &mut r);
        let peak = l.iter().chain(r.iter()).fold(0.0f32, |m, x| m.max(x.abs()));
        if peak > crate::dsp::CEIL {
            crate::dsp::CEIL / peak
        } else {
            1.0
        }
    };
    for bs in [n.max(1), 1, 7, 64, 333] {
        let mut sg = StreamGraph::try_from_doc(doc).unwrap();
        let (mut gl, mut gr) = (Vec::with_capacity(n), Vec::with_capacity(n));
        while gl.len() < n {
            let take = bs.min(n - gl.len());
            let (mut bl, mut br) = (vec![0.0f32; take], vec![0.0f32; take]);
            sg.fill_stereo(&mut bl, &mut br);
            gl.extend(bl);
            gr.extend(br);
        }
        let scaled = |v: &[f32]| v.iter().map(|x| x * gain).collect::<Vec<_>>();
        assert_eq!(bits(&scaled(&gl)), bits(&el), "left, block size {bs}");
        assert_eq!(bits(&scaled(&gr)), bits(&er), "right, block size {bs}");
    }
}

/// Mixer documents the block-size sweep runs over: every console feature
/// (lanes, sidechains, buses, sends, offsets, mutes, master chains) and the
/// shapes the golden suite pins.
const TRACKS_CORPUS: &[&str] = &[
    // The full console in one document: kit + saw + super + a tempo-mapped
    // seq, linear/step/exp automation lanes, a kick→bass sidechain, one bus
    // with inserts plus a send, and a master chain with a reverb (the 0/23
    // decorrelated pair) into a compressor. Hot faders, so the joint peak
    // limit bites and the probe gain is exercised too.
    r#"{ "name":"mix", "duration":1.0, "seed":11, "version":2, "engine":5,
        "root": { "type":"tracks",
          "buses": [ { "id":"verb", "gain":0.8, "effects": [
              { "type":"reverb", "room":0.6, "mix":0.5 },
              { "type":"lowpass", "cutoff":3200, "q":0.7 } ] } ],
          "tracks": [
            { "id":"kick", "node": { "type":"seq", "bpm":120, "steps_per_beat":4, "wave":"kit",
                "env": { "a":0.001, "d":0.08, "s":0.0, "r":0.04 },
                "notes": [ { "step":0, "len":1, "pitch":"midi:36" }, { "step":4, "len":1, "pitch":"midi:36" },
                           { "step":8, "len":1, "pitch":"midi:38" }, { "step":12, "len":1, "pitch":"midi:36" } ] },
              "gain":0.9 },
            { "id":"bass", "node": { "type":"chain", "stages": [
                  { "type":"sawtooth", "freq":55 }, { "type":"lowpass", "cutoff":400, "q":0.8 } ] },
              "gain":0.6, "at":0.013,
              "automation": [ { "target":"gain", "curve":"linear", "points": [
                  { "t":0.0, "v":0.2 }, { "t":0.7, "v":0.9 }, { "t":1.0, "v":0.4 } ] } ],
              "sidechain": { "source":"kick", "amount":0.75, "attack":0.004, "release":0.12 } },
            { "id":"pad", "node": { "type":"super", "wave":"sawtooth", "freq":220, "voices":5, "detune_cents":14 },
              "gain":0.3, "at":0.11, "pan":-0.4,
              "automation": [
                { "target":"gain", "curve":"exp", "points": [ { "t":0.05, "v":0.1 }, { "t":0.8, "v":0.5 } ] },
                { "target":"pan", "curve":"step", "points": [
                    { "t":0.0, "v":-0.6 }, { "t":0.3, "v":0.0 }, { "t":0.6, "v":0.6 } ] } ],
              "sends": [ { "bus":"verb", "amount":0.6 } ] },
            { "id":"arp", "node": { "type":"seq", "bpm":120, "steps_per_beat":4, "wave":"square",
                "tempo_map": [ { "at":{"num":0,"den":1}, "bpm":120 }, { "at":{"num":2,"den":1}, "bpm":90 } ],
                "env": { "a":0.004, "d":0.05, "s":0.4, "r":0.06 },
                "notes": [ { "step":0, "len":2, "pitch":"C4" }, { "step":2, "len":2, "pitch":"E4" },
                           { "step":4, "len":2, "pitch":"G4" }, { "step":6, "len":2, "pitch":"C5" } ] },
              "gain":0.25, "pan":0.5, "at":0.007 }
          ],
          "master": [
            { "type":"reverb", "room":0.35, "mix":0.18 },
            { "type":"compress", "threshold":-12, "ratio":2.5, "attack":0.006, "release":0.09, "makeup":1.5 } ] } }"#,
    // Muted tracks contribute exact zeros and draw nothing; a muted sidechain
    // source leaves the follower's envelope open; a missing source (an
    // unvalidated doc) means no ducking.
    r#"{ "name":"mut", "duration":0.5, "seed":7, "version":2, "engine":5,
        "root": { "type":"tracks", "tracks": [
            { "id":"kick", "node": { "type":"seq", "bpm":160, "steps_per_beat":4, "wave":"kit",
                "env": { "a":0.001, "d":0.08, "s":0.0, "r":0.04 },
                "notes": [ { "step":0, "len":1, "pitch":"midi:36" }, { "step":4, "len":1, "pitch":"midi:36" } ] },
              "gain":0.8 },
            { "id":"bass", "node": { "type":"sawtooth", "freq":55 }, "gain":0.5, "mute":true,
              "sidechain": { "source":"kick", "amount":0.8, "attack":0.005, "release":0.1 } },
            { "id":"ghost", "node": { "type":"noise", "color":"white" }, "mute":true },
            { "id":"pad", "node": { "type":"sine", "freq":440 }, "gain":0.4, "pan":-0.3,
              "sidechain": { "source":"ghost", "amount":0.8, "attack":0.005, "release":0.1 } },
            { "id":"lead", "node": { "type":"square", "freq":660 }, "gain":0.3, "pan":0.4,
              "sidechain": { "source":"nobody", "amount":0.8, "attack":0.005, "release":0.1 } }
        ] } }"#,
    // The three mixer shapes the golden suite pins.
    r#"{ "name": "tracks-sidechain", "duration": 1.0, "seed": 6, "version": 2, "engine": 5,
        "root": { "type": "tracks", "tracks": [
            { "id": "kick", "node": { "type": "seq", "bpm": 240, "wave": "kit", "kit": "808",
                "env": { "a": 0.001, "d": 0.1, "s": 0.5, "r": 0.1 },
                "notes": [ { "step": 0, "len": 1, "pitch": "midi:36" }, { "step": 4, "len": 1, "pitch": "midi:36" },
                           { "step": 8, "len": 1, "pitch": "midi:36" }, { "step": 12, "len": 1, "pitch": "midi:36" } ] } },
            { "id": "bass", "node": { "type": "seq", "bpm": 240, "wave": "sawtooth",
                "env": { "a": 0.005, "d": 0.05, "s": 0.8, "r": 0.05 },
                "notes": [ { "step": 0, "len": 16, "pitch": "C2" } ] },
              "sidechain": { "source": "kick", "amount": 0.8, "attack": 0.005, "release": 0.15 } }
        ] } }"#,
    r#"{ "name": "tracks-automation-curves", "duration": 1.0, "seed": 9, "version": 2, "engine": 5,
        "root": { "type": "tracks", "tracks": [
            { "id": "pad", "node": { "type": "sawtooth", "freq": 220 }, "gain": 0.5,
              "automation": [
                { "target": "gain", "curve": "exp", "points": [ { "t": 0.0, "v": 0.2 }, { "t": 0.6, "v": 0.9 } ] },
                { "target": "pan", "curve": "step", "points": [ { "t": 0.0, "v": -0.5 }, { "t": 0.5, "v": 0.5 } ] }
              ] }
        ] } }"#,
    r#"{ "name": "tracks-bus-mix", "duration": 1.0, "seed": 6, "version": 2, "engine": 5,
        "root": { "type": "tracks",
            "buses": [ { "id": "verb", "gain": 0.8, "effects": [ { "type": "reverb", "room": 0.5, "mix": 0.6 } ] } ],
            "tracks": [
                { "id": "kick", "node": { "type": "seq", "bpm": 240, "wave": "kit", "kit": "808",
                    "env": { "a": 0.001, "d": 0.1, "s": 0.5, "r": 0.1 },
                    "notes": [ { "step": 0, "len": 1, "pitch": "midi:36" } ] },
                  "sends": [ { "bus": "verb", "amount": 0.3 } ] },
                { "id": "pad", "node": { "type": "sawtooth", "freq": 110 }, "gain": 0.4, "pan": 0.3, "bus": "verb" }
            ] } }"#,
    // Non-reverb bus/master inserts run as per-channel pairs.
    r#"{ "name":"md", "duration":0.4, "seed":2, "version":2, "engine":5,
        "root": { "type":"tracks",
          "tracks": [
            { "id":"a", "node": { "type":"sawtooth", "freq":110 }, "gain":0.5, "pan":-0.5 },
            { "id":"b", "node": { "type":"noise", "color":"brown" }, "gain":0.3, "pan":0.6, "bus":"fx" }
          ],
          "buses": [ { "id":"fx", "gain":1.1, "effects": [
              { "type":"delay", "secs":0.02, "feedback":0.35 },
              { "type":"chorus", "rate":1.2, "depth":0.5, "mix":0.4 } ] } ],
          "master": [
              { "type":"delay", "secs":0.011, "feedback":0.25 },
              { "type":"compress", "threshold":-12, "ratio":3, "attack":0.003, "release":0.05, "makeup":1 } ] } }"#,
    // A modulated filter inside a layer.
    r#"{ "name":"c", "duration":0.1, "version":2, "engine":5, "root": { "type":"tracks", "tracks": [
        { "id":"lead", "node": { "type":"chain", "stages": [
            { "type":"sawtooth", "freq":110 },
            { "type":"lowpass", "cutoff": { "lfo": { "rate":2, "depth":400, "center":800 } } } ] } } ] } }"#,
    // The console at engine 5: exp lanes (det powf on the lane cursor), a
    // sidechain (det exp on the follower), a reverb bus, a master compressor.
    r#"{ "name":"e5mix", "duration":0.8, "seed":6, "version":2, "engine":5,
        "root": { "type":"tracks",
          "buses": [ { "id":"verb", "gain":0.8, "effects": [ { "type":"reverb", "room":0.5, "mix":0.4 } ] } ],
          "tracks": [
            { "id":"keys", "node": { "type":"seq", "bpm":120, "steps_per_beat":4, "wave":"epiano",
                "env": { "a":0.003, "d":0.1, "s":0.5, "r":0.1 },
                "notes": [ { "step":0, "len":4, "pitch":"C4" }, { "step":4, "len":4, "pitch":"G3", "gain":0.8 } ] },
              "gain":0.7, "pan":-0.3,
              "automation": [ { "target":"gain", "curve":"exp", "points": [ { "t":0.0, "v":0.3 }, { "t":0.7, "v":0.9 } ] } ],
              "sends": [ { "bus":"verb", "amount":0.4 } ] },
            { "id":"bass", "node": { "type":"seq", "bpm":120, "steps_per_beat":4, "wave":"bass",
                "env": { "a":0.004, "d":0.06, "s":0.8, "r":0.06 },
                "notes": [ { "step":0, "len":8, "pitch":"C2" } ] },
              "gain":0.6, "at":0.05, "pan":0.2,
              "sidechain": { "source":"keys", "amount":0.6, "attack":0.005, "release":0.12 } }
          ],
          "master": [ { "type":"compress", "threshold":-14, "ratio":2.5, "attack":0.005, "release":0.08, "makeup":1.5 } ] } }"#,
];

#[test]
fn every_mixer_shape_streams_byte_identically() {
    for json in TRACKS_CORPUS {
        let doc = parse(json);
        assert!(StreamGraph::blockers(&doc).is_empty(), "{json}");
        assert_tracks_byte_identical(&doc);
    }
    // The `at` matrix: tracks landing at different song positions (including
    // after lanes have started moving) reproduce the render-then-shift exactly.
    for at in [(0.0, 0.013, 0.11), (0.21, 0.0, 0.047), (0.5, 0.5, 0.0)] {
        assert_tracks_byte_identical(&parse(&format!(
            r#"{{ "name":"offs", "duration":0.6, "seed":5, "version":2, "engine":5,
                "root": {{ "type":"tracks", "tracks": [
                    {{ "id":"hiss", "node": {{ "type":"noise", "color":"pink" }}, "gain":0.4, "at":{} }},
                    {{ "id":"tone", "node": {{ "type":"triangle", "freq":330 }}, "gain":0.5, "at":{},
                      "automation": [ {{ "target":"pan", "curve":"linear", "points": [
                          {{ "t":0.0, "v":-0.8 }}, {{ "t":0.6, "v":0.8 }} ] }} ] }},
                    {{ "id":"blip", "node": {{ "type":"chain", "stages": [
                          {{ "type":"square", "freq":880 }}, {{ "type":"drive", "amount":3, "shape":"hard" }} ] }},
                      "gain":0.3, "at":{} }}
                ] }} }}"#,
            at.0, at.1, at.2
        )));
    }
}

#[test]
fn tracks_mono_fill_is_the_mid() {
    // The mono view of a streamed mixer is its mid — what render_product
    // hands mono consumers.
    let doc = parse(
        r#"{ "name":"mid", "duration":0.2, "seed":4, "version":2, "engine":5,
            "root": { "type":"tracks", "tracks": [
                { "id":"l", "node": { "type":"sine", "freq":220 }, "pan":-0.7, "gain":0.5 },
                { "id":"r", "node": { "type":"sine", "freq":330 }, "pan":0.7, "gain":0.5 } ] } }"#,
    );
    let product = crate::render::render_product(&doc);
    let mut sg = StreamGraph::try_from_doc(&doc).unwrap();
    let mut got = vec![0.0f32; product.mono.len()];
    sg.fill(&mut got);
    // The mono mid precedes the peak limit; scale by the probe gain like the
    // stereo helper does (this doc is quiet — gain 1.0, but keep the shape).
    let peak = {
        let mut probe = StreamGraph::try_from_doc(&doc).unwrap();
        let (mut l, mut r) = (vec![0.0f32; got.len()], vec![0.0f32; got.len()]);
        probe.fill_stereo(&mut l, &mut r);
        l.iter().chain(r.iter()).fold(0.0f32, |m, x| m.max(x.abs()))
    };
    let gain = if peak > crate::dsp::CEIL {
        crate::dsp::CEIL / peak
    } else {
        1.0
    };
    let scaled: Vec<f32> = got.iter().map(|x| x * gain).collect();
    assert_eq!(bits(&scaled), bits(&product.mono), "mono fill is the mid");
}

#[test]
fn tracks_blockers_name_the_failing_part_with_context() {
    let part = |part: &str, cause: StreamBlocker| StreamBlocker::TracksPart {
        part: part.to_string(),
        cause: Box::new(cause),
    };
    let cases: &[(&str, StreamBlocker)] = &[
        // A sampler track names the track.
        (
            r#"{ "name":"b", "duration":0.1, "version":2, "engine":5, "root": { "type":"tracks", "tracks": [
                { "id":"keys", "node": { "type":"seq", "wave":"sampler", "sf2":"x.sf2", "bpm":100,
                    "env": { "a":0.001, "s":1.0, "r":0.1 },
                    "notes": [ { "step":0, "len":4, "pitch":"C4" } ] } } ] } }"#,
            part("track 'keys'", StreamBlocker::Sampler),
        ),
    ];
    for (json, want) in cases {
        let doc = parse(json);
        let got = StreamGraph::blockers(&doc);
        assert!(got.contains(want), "{json}: got {got:?}");
        assert!(StreamGraph::try_from_doc(&doc).is_none(), "{json}");
        // The message carries both the context and the cause's fix.
        let msg = want.to_string();
        assert!(msg.contains('—'), "{msg}");
        if let StreamBlocker::TracksPart { part, .. } = want {
            assert!(msg.contains(part), "{msg}");
        }
    }
}

#[test]
fn tracks_doc_level_blockers_still_fire() {
    // normalize / loop / stereo treatments stay whole-buffer blockers even
    // though the mixer itself streams.
    let cases: &[(&str, StreamBlocker)] = &[
        (
            r#"{ "name":"n", "duration":0.1, "version":2, "engine":5,
                "normalize": { "target_lufs": -14 },
                "root": { "type":"tracks", "tracks": [ { "node": { "type":"sine", "freq":440 } } ] } }"#,
            StreamBlocker::Normalize,
        ),
        (
            r#"{ "name":"l", "duration":0.5, "version":2, "engine":5,
                "playback": { "mode":"loop", "start_secs":0.1, "crossfade_secs":0.05 },
                "root": { "type":"tracks", "tracks": [ { "node": { "type":"sine", "freq":220 } } ] } }"#,
            StreamBlocker::LoopPlayback,
        ),
        (
            r#"{ "name":"s", "duration":0.1, "version":2, "engine":5,
                "stereo": { "mode":"haas", "ms":12 },
                "root": { "type":"tracks", "tracks": [ { "node": { "type":"sine", "freq":220 } } ] } }"#,
            StreamBlocker::StereoTreatment,
        ),
    ];
    for (json, want) in cases {
        let doc = parse(json);
        let got = StreamGraph::blockers(&doc);
        assert_eq!(got, vec![want.clone()], "{json}");
        assert!(StreamGraph::try_from_doc(&doc).is_none(), "{json}");
    }
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
        9 => json!({ "type":"tremolo", "rate":rf(rng,1.0,12.0), "depth":rf(rng,0.2,0.9) }),
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
            json!({ "name":"fuzz", "duration": dur, "seed": seed, "engine": 5, "root": root });
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
fn render_is_bit_deterministic_in_process() {
    // Twice in one process, identical bits. The cross-platform half of the
    // promise holds by construction: every transcendental in this render goes
    // through crate::det (the f32 wrappers and the f64 gated-loudness path)
    // — no platform libm remains in the engine-5 path. The CI multi-platform
    // run is the real proof; this pins the per-process determinism plus the
    // output stage.
    let doc = parse(
        r#"{ "name":"e5all", "duration":0.5, "seed":9, "version":2, "engine":5,
            "normalize": { "target_lufs": -16, "ceiling_dbtp": -1.0 },
            "root": { "type":"chain", "stages": [
                { "type":"mul", "inputs": [
                    { "type":"fm", "freq":"A3", "ratio":2.5, "index":3 },
                    { "type":"env", "a":0.005, "d":0.1, "s":0.4, "r":0.1 } ] },
                { "type":"reverb", "room":0.5, "mix":0.35 },
                { "type":"chorus", "rate":1.1, "depth":0.5, "mix":0.4 } ] } }"#,
    );
    doc.validate().unwrap();
    let a = crate::render::render_product(&doc);
    let b = crate::render::render_product(&doc);
    assert_eq!(bits(&a.mono), bits(&b.mono));
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

#[test]
fn blockers_names_every_reason_with_a_fix() {
    let cases: &[(&str, StreamBlocker)] = &[
        (
            r#"{ "name":"n", "duration":0.1, "normalize": { "target_lufs": -14 },
                "root": { "type":"sine", "freq":440 } }"#,
            StreamBlocker::Normalize,
        ),
        (
            r#"{ "name":"l", "duration":0.5,
                "playback": { "mode":"loop", "start_secs":0.1, "crossfade_secs":0.05 },
                "root": { "type":"sine", "freq":220 } }"#,
            StreamBlocker::LoopPlayback,
        ),
        (
            r#"{ "name":"s", "duration":0.1, "stereo": { "mode":"haas", "ms":12 },
                "root": { "type":"sine", "freq":220 } }"#,
            StreamBlocker::StereoTreatment,
        ),
        (
            r#"{ "name":"sf", "duration":0.1, "engine":5, "root": { "type":"seq",
                "wave":"sampler", "sf2":"x.sf2", "bpm":100, "steps":4,
                "env": { "a":0.001, "s":1.0, "r":0.1 },
                "notes": [ { "step":0, "len":4, "pitch":"C4" } ] } }"#,
            StreamBlocker::Sampler,
        ),
    ];
    for (json, want) in cases {
        let doc = parse(json);
        let got = StreamGraph::blockers(&doc);
        assert!(got.contains(want), "{json}: got {got:?}");
        assert!(StreamGraph::try_from_doc(&doc).is_none(), "{json}");
        // Every blocker message carries the fix, not just the fault.
        assert!(want.to_string().contains('—'), "{want}");
    }
}

#[test]
fn blockers_agrees_with_try_from_doc() {
    // The report and the silent Option must never disagree: streamable docs
    // report no blockers, blocked docs report at least one.
    let streamable = [
        r#"{ "name":"a", "duration":0.1, "root": { "type":"sine", "freq":440 } }"#,
        r#"{ "name":"b", "duration":0.1, "engine":5, "seed":3, "root": { "type":"chain", "stages": [
            { "type":"noise", "color":"pink" },
            { "type":"lowpass", "cutoff":1200, "q":0.8 },
            { "type":"reverb", "room":0.4, "mix":0.3 } ] } }"#,
        r#"{ "name":"c", "duration":0.1, "root": { "type":"mul", "inputs": [
            { "type":"fm", "freq":440, "ratio":2.0, "index": { "slide": { "from":4, "to":1, "secs":0.08 } } },
            { "type":"env", "a":0.001, "d":0.09, "s":0.0, "r":0.03 } ] } }"#,
        // A mixer with automation, a sidechain, and a bus streams.
        r#"{ "name":"tv", "duration":0.2, "version":2, "engine":5, "seed":3,
            "root": { "type":"tracks",
              "buses": [ { "id":"verb", "gain":0.8, "effects": [ { "type":"reverb", "room":0.4, "mix":0.3 } ] } ],
              "tracks": [
                { "id":"kick", "node": { "type":"seq", "bpm":120, "steps_per_beat":4, "wave":"square",
                    "env": { "a":0.001, "d":0.05, "s":0.4, "r":0.05 },
                    "notes": [ { "step":0, "len":2, "pitch":"C3" } ] },
                  "sends": [ { "bus":"verb", "amount":0.4 } ] },
                { "id":"bass", "node": { "type":"chain", "stages": [
                      { "type":"sawtooth", "freq":55 }, { "type":"lowpass", "cutoff":500, "q":0.8 } ] },
                  "gain":0.7, "at":0.01,
                  "automation": [ { "target":"gain", "points": [ { "t":0.0, "v":0.3 }, { "t":0.2, "v":0.7 } ] } ],
                  "sidechain": { "source":"kick", "amount":0.7, "attack":0.005, "release":0.1 } }
              ],
              "master": [ { "type":"reverb", "room":0.3, "mix":0.2 } ] } }"#,
    ];
    for json in streamable {
        let doc = parse(json);
        doc.validate().unwrap();
        assert_eq!(StreamGraph::blockers(&doc), Vec::new(), "{json}");
        assert!(StreamGraph::try_from_doc(&doc).is_some(), "{json}");
    }
    let blocked = [
        r#"{ "name":"f", "duration":0.1, "normalize": { "ceiling_dbtp": -1.0 },
            "root": { "type":"sine", "freq":440 } }"#,
        r#"{ "name":"g", "duration":0.1, "engine":5, "root": { "type":"seq",
            "wave":"sampler", "sf2":"x.sf2", "bpm":100,
            "env": { "a":0.001, "s":1.0, "r":0.1 },
            "notes": [ { "step":0, "len":4, "pitch":"C4" } ] } }"#,
        // A mixer with an unstreamable part reports the part and rejects.
        r#"{ "name":"h", "duration":0.1, "version":2, "engine":5, "root": { "type":"tracks", "tracks": [
            { "id":"keys", "node": { "type":"seq", "wave":"sampler", "sf2":"x.sf2", "bpm":100,
                "env": { "a":0.001, "s":1.0, "r":0.1 },
                "notes": [ { "step":0, "len":4, "pitch":"C4" } ] } } ] } }"#,
    ];
    for json in blocked {
        let doc = parse(json);
        assert!(!StreamGraph::blockers(&doc).is_empty(), "{json}");
        assert!(StreamGraph::try_from_doc(&doc).is_none(), "{json}");
    }
}

#[test]
fn multiple_blockers_all_report_once() {
    // A doc tripping several rules reports each once, doc-level first.
    let doc = parse(
        r#"{ "name":"x", "duration":0.1, "engine":5,
            "normalize": { "target_lufs": -14 },
            "stereo": { "mode":"wide" },
            "root": { "type":"mix", "inputs": [
                { "type":"noise", "color":"white" },
                { "type":"dust", "density":30, "decay":0.02 } ] } }"#,
    );
    let got = StreamGraph::blockers(&doc);
    assert_eq!(
        got,
        vec![StreamBlocker::Normalize, StreamBlocker::StereoTreatment,]
    );
}
