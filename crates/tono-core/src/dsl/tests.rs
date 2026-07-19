use super::*;
use crate::dsl::ValidateError;

fn roundtrip(json: &str) -> serde_json::Value {
    let doc: SoundDoc = serde_json::from_str(json).expect("deserialize");
    serde_json::to_value(&doc).expect("serialize")
}

fn doc_with_root(root: &str) -> SoundDoc {
    serde_json::from_str(&format!(
        r#"{{ "name": "t", "duration": 0.2, "engine": 1, "root": {root} }}"#
    ))
    .expect("deserialize")
}

#[test]
fn doc_defaults_fill_in() {
    let doc: SoundDoc =
        serde_json::from_str(r#"{ "name": "beep", "root": { "type": "sine", "freq": 440 } }"#)
            .unwrap();
    assert_eq!(doc.duration, 0.3);
    assert_eq!(doc.sample_rate, 44_100);
    // Version-less documents keep the pre-versioning (v1) render semantics.
    assert_eq!(doc.version, None);
    assert_eq!(doc.effective_version(), 1);
    assert!(matches!(doc.stereo, Stereo::Mono));
    assert!(matches!(doc.playback, Playback::OneShot));
}

#[test]
fn v2_tracks_reject_doc_level_stereo() {
    let mut doc: SoundDoc = serde_json::from_str(
        r#"{ "name": "band", "duration": 0.2, "version": 2,
                "stereo": { "mode": "wide" },
                "root": { "type": "tracks",
                  "tracks": [ { "id": "a", "node": { "type": "sine", "freq": 220 } } ] } }"#,
    )
    .unwrap();
    let err = doc.validate().unwrap_err();
    assert!(err.contains("per-layer pan"), "{err}");
    // v1 documents keep the historical silent-ignore so old libraries load.
    doc.version = None;
    assert_eq!(doc.validate(), Ok(()));
}

#[test]
fn future_schema_versions_are_rejected() {
    let mut doc: SoundDoc =
        serde_json::from_str(r#"{ "name": "beep", "root": { "type": "sine", "freq": 440 } }"#)
            .unwrap();
    assert_eq!(doc.validate(), Ok(()));
    doc.version = Some(SCHEMA_VERSION);
    assert_eq!(doc.validate(), Ok(()));
    doc.version = Some(SCHEMA_VERSION + 1);
    let err = doc.validate().unwrap_err();
    assert!(err.contains("upgrade tono"), "unhelpful error: {err}");
    doc.version = Some(0);
    assert!(doc.validate().is_err());
}

#[test]
fn engine_defaults_to_zero_and_bounds_at_current_revision() {
    let mut doc: SoundDoc =
        serde_json::from_str(r#"{ "name": "beep", "root": { "type": "sine", "freq": 440 } }"#)
            .unwrap();
    // Omitted ⇒ engine 0 (the original kernels; existing docs stay bit-exact).
    assert_eq!(doc.engine, None);
    assert_eq!(doc.effective_engine(), 0);
    assert_eq!(doc.validate(), Ok(()));
    doc.engine = Some(ENGINE_VERSION);
    assert_eq!(doc.validate(), Ok(()));
    // A document from a newer DSP kernel is rejected, not misrendered.
    doc.engine = Some(ENGINE_VERSION + 1);
    let err = doc.validate().unwrap_err();
    assert!(err.contains("engine must be in"), "unhelpful error: {err}");
}

#[test]
fn modal_and_impact_validate_their_ranges() {
    let modal = |modes: &str| -> Result<(), ValidateError> {
        doc_with_root(&format!(
            r#"{{ "type": "chain", "stages": [
                    {{ "type": "impact" }},
                    {{ "type": "modal", "modes": {modes} }} ] }}"#
        ))
        .validate()
    };
    assert!(modal(r#"[ { "freq": 440, "decay": 0.5, "gain": 1.0 } ]"#).is_ok());
    assert!(modal("[]").unwrap_err().contains("non-empty"));
    assert!(modal(r#"[ { "freq": -1 } ]"#).unwrap_err().contains("freq"));
    assert!(
        modal(r#"[ { "freq": 440, "decay": 0 } ]"#)
            .unwrap_err()
            .contains("decay")
    );
    assert!(
        modal(r#"[ { "freq": 440, "gain": 2 } ]"#)
            .unwrap_err()
            .contains("gain")
    );
    // Impact ranges.
    assert!(
        doc_with_root(r#"{ "type": "impact", "hardness": 1.5 }"#)
            .validate()
            .unwrap_err()
            .contains("hardness")
    );
}

#[test]
fn dust_and_rand_validate_their_ranges() {
    // dust: density must be positive, decay non-negative.
    assert!(
        doc_with_root(r#"{ "type": "dust", "density": 50 }"#)
            .validate()
            .is_ok()
    );
    assert!(
        doc_with_root(r#"{ "type": "dust", "density": 0 }"#)
            .validate()
            .unwrap_err()
            .contains("density")
    );
    // rand modulator: rate must be positive.
    let with_cutoff = |m: &str| {
        doc_with_root(&format!(
            r#"{{ "type": "chain", "stages": [
                    {{ "type": "noise" }},
                    {{ "type": "lowpass", "cutoff": {m} }} ] }}"#
        ))
        .validate()
    };
    assert!(with_cutoff(r#"{ "rand": { "from": 200, "to": 1200, "rate": 0.8 } }"#).is_ok());
    assert!(
        with_cutoff(r#"{ "rand": { "from": 200, "to": 1200, "rate": 0 } }"#)
            .unwrap_err()
            .contains("rand.rate")
    );
}

#[test]
fn node_tag_is_type_lowercase() {
    let v = roundtrip(r#"{ "name": "n", "root": { "type": "ringmod", "freq": 100 } }"#);
    assert_eq!(v["root"]["type"], "ringmod");
}

#[test]
fn env_flattens_adsr_fields_inline() {
    // The wire shape keeps a/d/s/r/punch inline on the env node — the
    // internal Adsr struct must stay invisible to the JSON.
    let v = roundtrip(
        r#"{ "name": "n", "root": { "type": "env", "a": 0.01, "d": 0.2, "punch": 0.3 } }"#,
    );
    assert_eq!(v["root"]["a"], 0.01f32 as f64);
    assert_eq!(v["root"]["punch"], 0.3f32 as f64);
    assert!(v["root"].get("adsr").is_none());
}

#[test]
fn value_untagged_forms() {
    let doc: SoundDoc = serde_json::from_str(
        r#"{ "name": "n", "root": { "type": "mix", "inputs": [
                { "type": "sine", "freq": 440 },
                { "type": "sine", "freq": "A4" },
                { "type": "sine", "freq": { "slide": { "from": 880, "to": 180, "secs": 0.2 } } },
                { "type": "sine", "freq": { "lfo": { "rate": 5, "depth": 10, "center": 440 } } },
                { "type": "sine", "freq": { "arp": { "steps": [523, 659], "rate": 12 } } },
                { "type": "sine", "freq": { "env": { "a": 0.1, "from": 100, "to": 800 } } }
            ] } }"#,
    )
    .unwrap();
    let Node::Mix { inputs } = &doc.root else {
        panic!("expected mix");
    };
    assert!(matches!(
        &inputs[0],
        Node::Sine {
            freq: Value::Const(f)
        } if *f == 440.0
    ));
    assert!(matches!(&inputs[1], Node::Sine { freq: Value::Note(s) } if s == "A4"));
    assert!(matches!(
        &inputs[2],
        Node::Sine {
            freq: Value::Modulated(Modulator::Slide {
                curve: Curve::Lin,
                ..
            })
        }
    ));
    assert!(matches!(
        &inputs[5],
        Node::Sine {
            freq: Value::Modulated(Modulator::EnvMod { adsr, .. })
        } if adsr.a == 0.1
    ));
}

#[test]
fn playback_loop_tag() {
    let v = roundtrip(
        r#"{ "name": "n", "playback": { "mode": "loop", "crossfade_secs": 0.25 },
                 "root": { "type": "noise" } }"#,
    );
    assert_eq!(v["playback"]["mode"], "loop");
    assert_eq!(v["playback"]["crossfade_secs"], 0.25f32 as f64);
}

#[test]
fn stereo_modes() {
    let v = roundtrip(
        r#"{ "name": "n", "stereo": { "mode": "haas", "pan": -1 },
                 "root": { "type": "noise" } }"#,
    );
    assert_eq!(v["stereo"]["mode"], "haas");
    assert_eq!(v["stereo"]["ms"], 12.0); // default filled in
}

#[test]
fn note_names_resolve_to_hz() {
    assert_eq!(note_to_hz("A4"), Some(440.0));
    assert_eq!(note_to_hz("midi:69"), Some(440.0));
    assert_eq!(note_to_hz("m69"), Some(440.0));
    // C#3 = midi 49 ≈ 138.59 Hz; Gb5 = midi 78 ≈ 739.99 Hz.
    assert!((note_to_hz("C#3").unwrap() - 138.591).abs() < 0.01);
    assert!((note_to_hz("Gb5").unwrap() - 739.989).abs() < 0.01);
    // Octave defaults to 4; accidentals stack; case-insensitive letter.
    assert_eq!(note_to_hz("A"), Some(440.0));
    assert_eq!(note_to_hz("a4"), Some(440.0));
    assert!((note_to_hz("F#-1").unwrap() - 11.562).abs() < 0.01);
    // Garbage stays unparsed.
    assert_eq!(note_to_hz(""), None);
    assert_eq!(note_to_hz("H4"), None);
    assert_eq!(note_to_hz("A4x"), None);
}

#[test]
fn processors_are_processors_sources_are_not() {
    let p: Node = serde_json::from_str(r#"{ "type": "reverb", "room": 0.5 }"#).unwrap();
    assert!(p.is_processor());
    let s: Node = serde_json::from_str(r#"{ "type": "sine", "freq": 440 }"#).unwrap();
    assert!(!s.is_processor());
}

fn doc(json: &str) -> SoundDoc {
    serde_json::from_str(json).expect("deserialize")
}

#[test]
fn validate_accepts_a_sane_doc() {
    let d = doc(
        r#"{ "name": "zap", "duration": 0.2, "root": { "type": "mul", "inputs": [
                { "type": "square", "freq": { "slide": { "from": 880, "to": 180, "secs": 0.18 } } },
                { "type": "env", "d": 0.18, "punch": 0.3 }
            ] } }"#,
    );
    assert_eq!(d.validate(), Ok(()));
}

#[test]
fn validate_rejects_out_of_range_metadata() {
    let d = doc(r#"{ "name": "n", "duration": 0, "root": { "type": "noise" } }"#);
    assert!(d.validate().unwrap_err().contains("duration"));
    let d = doc(r#"{ "name": "n", "sample_rate": 1000, "root": { "type": "noise" } }"#);
    assert!(d.validate().unwrap_err().contains("sample_rate"));
    let d =
        doc(r#"{ "name": "n", "normalize": { "target_lufs": 5 }, "root": { "type": "noise" } }"#);
    assert!(d.validate().unwrap_err().contains("target_lufs"));
}

#[test]
fn validate_rejects_bad_loop_region() {
    let d = doc(r#"{ "name": "n", "duration": 1,
                 "playback": { "mode": "loop", "start_secs": 0.8, "end_secs": 0.5 },
                 "root": { "type": "noise" } }"#);
    assert!(d.validate().unwrap_err().contains("end_secs"));
}

#[test]
fn validate_rejects_unit_range_violations() {
    let d = doc(r#"{ "name": "n", "root": { "type": "chain", "stages": [
                { "type": "noise" }, { "type": "reverb", "mix": 1.5 }
            ] } }"#);
    assert!(d.validate().unwrap_err().contains("reverb.mix"));
    let d = doc(r#"{ "name": "n", "root": { "type": "env", "s": 2 } }"#);
    assert!(d.validate().unwrap_err().contains("env.s"));
}

#[test]
fn validate_rejects_silent_all_zero_env() {
    // The flatten footgun: nesting a/d/s/r under an "adsr" object silently
    // drops them all to 0, so the env renders pure silence.
    let d = doc(r#"{ "name": "n", "root": { "type": "env",
                "adsr": { "a": 0.01, "d": 0.1, "s": 0.7, "r": 0.2 } } }"#);
    assert!(d.validate().unwrap_err().contains("env is silent"));
    // Correctly inlined, it validates.
    let ok = doc(
        r#"{ "name": "n", "root": { "type": "env", "a": 0.01, "d": 0.1, "s": 0.7, "r": 0.2 } }"#,
    );
    assert!(ok.validate().is_ok());
}

#[test]
fn validate_rejects_extreme_eq_gain() {
    // Beyond ±24 dB the biquad coefficients blow up to inf/NaN.
    let d = doc(r#"{ "name": "n", "root": { "type": "chain", "stages": [
                { "type": "noise" }, { "type": "peak", "cutoff": 1000, "gain_db": 2000 }
            ] } }"#);
    assert!(d.validate().unwrap_err().contains("peak.gain_db"));
    let d = doc(r#"{ "name": "n", "root": { "type": "chain", "stages": [
                { "type": "noise" }, { "type": "lowshelf", "cutoff": 200, "gain_db": -100 }
            ] } }"#);
    assert!(d.validate().unwrap_err().contains("shelf.gain_db"));
}

#[test]
fn validate_rejects_empty_combinators_and_bad_notes() {
    let d = doc(r#"{ "name": "n", "root": { "type": "mix", "inputs": [] } }"#);
    assert!(d.validate().unwrap_err().contains("mix/mul"));
    let d = doc(r#"{ "name": "n", "root": { "type": "sine", "freq": "H9" } }"#);
    assert!(d.validate().unwrap_err().contains("not a valid note"));
}

#[test]
fn validate_bounds_the_delay_line() {
    // Unbounded delay.secs would let a validated doc request an arbitrary
    // allocation and abort the process.
    let d = doc(r#"{ "name": "n", "root": { "type": "chain", "stages": [
                { "type": "noise" }, { "type": "delay", "secs": 1e9, "feedback": 0.3 }
            ] } }"#);
    assert!(d.validate().unwrap_err().contains("delay.secs"));
    let ok = doc(r#"{ "name": "n", "root": { "type": "chain", "stages": [
                { "type": "noise" }, { "type": "delay", "secs": 0.3, "feedback": 0.3 }
            ] } }"#);
    assert!(ok.validate().is_ok());
}

#[test]
fn validate_rejects_non_finite_and_non_positive_constants() {
    // 1e308 overflows the silent f64→f32 cast to inf, which renders NaN.
    let d = doc(r#"{ "name": "n", "root": { "type": "sine", "freq": 1e308 } }"#);
    assert!(d.validate().unwrap_err().contains("finite"));
    let d = doc(r#"{ "name": "n", "root": { "type": "sine", "freq": -440 } }"#);
    assert!(d.validate().unwrap_err().contains("sine.freq"));
    let d = doc(r#"{ "name": "n", "root": { "type": "square", "duty": 0.5,
                "freq": { "slide": { "from": 1e308, "to": 440, "secs": 0.1 } } } }"#);
    assert!(d.validate().unwrap_err().contains("slide.from"));
}

#[test]
fn validate_rejects_non_finite_seq_voice_knobs() {
    let d = doc(
        r#"{ "name": "n", "root": { "type": "seq", "bpm": 120, "wave": "bass",
                "bass_cutoff": 1e308,
                "env": { "a": 0.005, "d": 0.05, "s": 0.7, "r": 0.1 },
                "notes": [ { "step": 0, "len": 2, "pitch": "E1" } ] } }"#,
    );
    assert!(d.validate().unwrap_err().contains("bass_cutoff"));
}

#[test]
fn validate_checks_automation_lanes() {
    let d = doc(
        r#"{ "name": "n", "version": 2, "root": { "type": "tracks", "tracks": [
                { "id": "a", "node": { "type": "noise" },
                  "automation": [ { "target": "gain", "points": [ { "t": -1, "v": 0.5 } ] } ] }
            ] } }"#,
    );
    assert!(d.validate().unwrap_err().contains(".t must be >= 0"));
    let d = doc(
        r#"{ "name": "n", "version": 2, "root": { "type": "tracks", "tracks": [
                { "id": "a", "node": { "type": "noise" },
                  "automation": [ { "target": "pan", "points": [ { "t": 0, "v": 7 } ] } ] }
            ] } }"#,
    );
    assert!(d.validate().unwrap_err().contains("[-1, 1]"));
}

#[test]
fn validate_rejects_pitches_that_resolve_non_finite() {
    // midi:10000 → 440·2^827.6 = f32 inf; the oscillator phase would go NaN.
    let d = doc(r#"{ "name": "n", "root": { "type": "sine", "freq": "midi:10000" } }"#);
    assert!(d.validate().unwrap_err().contains("not a valid note"));
    assert_eq!(note_to_hz("midi:10000"), None);
    assert_eq!(note_to_hz("midi:-100000"), None);
    // A huge octave must not panic the parser (i32 overflow) — just reject.
    let d = doc(r#"{ "name": "n", "root": { "type": "sine", "freq": "A200000000" } }"#);
    assert!(d.validate().unwrap_err().contains("not a valid note"));
    assert_eq!(note_to_hz("A200000000"), None);
}

#[test]
fn validate_rejects_overflow_regime_knobs() {
    // 2^(cents/1200) must stay far from f32 overflow or the voices render NaN.
    let d =
        doc(r#"{ "name": "n", "root": { "type": "super", "freq": 110, "detune_cents": 200000 } }"#);
    assert!(d.validate().unwrap_err().contains("detune_cents"));
    // fm.freq × fm.ratio must stay far from f32 overflow for the same reason.
    let d =
        doc(r#"{ "name": "n", "root": { "type": "fm", "freq": 440, "ratio": 1e20, "index": 1 } }"#);
    assert!(d.validate().unwrap_err().contains("fm.ratio"));
    let d =
        doc(r#"{ "name": "n", "root": { "type": "fm", "freq": 1e20, "ratio": 2.0, "index": 1 } }"#);
    assert!(d.validate().unwrap_err().contains("fm.freq"));
    let d = doc(
        r#"{ "name": "n", "root": { "type": "fm", "ratio": 2.0, "index": 1,
                "freq": { "slide": { "from": 1e20, "to": 440, "secs": 0.1 } } } }"#,
    );
    assert!(d.validate().unwrap_err().contains("slide.from"));
}

#[test]
fn validate_caps_rand_rate() {
    // Past the cap the walk is indistinguishable from noise, and the
    // renderer's per-sample catch-up loop becomes a denial of service.
    let d = doc(r#"{ "name": "n", "root": { "type": "chain", "stages": [
                { "type": "noise" },
                { "type": "lowpass", "cutoff": { "rand": { "from": 200, "to": 1200, "rate": 1e12 } } }
            ] } }"#);
    assert!(d.validate().unwrap_err().contains("rand.rate"));
    let ok = doc(r#"{ "name": "n", "root": { "type": "chain", "stages": [
                { "type": "noise" },
                { "type": "lowpass", "cutoff": { "rand": { "from": 200, "to": 1200, "rate": 9000 } } }
            ] } }"#);
    assert!(ok.validate().is_ok());
}

#[test]
fn validate_rejects_non_finite_compress_ratio() {
    // 1e308 overflows the silent f64→f32 cast to inf — it used to slip past
    // the ratio >= 1 check.
    let d = doc(r#"{ "name": "n", "root": { "type": "chain", "stages": [
                { "type": "noise" }, { "type": "compress", "threshold": 0.5, "ratio": 1e308 }
            ] } }"#);
    assert!(d.validate().unwrap_err().contains("compress.ratio"));
}

#[test]
fn validate_rejects_silent_processor_positions() {
    // A chain leading with a processor has no input and renders silence.
    let d = doc(r#"{ "name": "n", "root": { "type": "chain", "stages": [
                { "type": "lowpass", "cutoff": 800 }, { "type": "sine", "freq": 440 }
            ] } }"#);
    assert!(d.validate().unwrap_err().contains("first stage"));
    // Same for a bare-processor document root.
    let d = doc(r#"{ "name": "n", "root": { "type": "lowpass", "cutoff": 800 } }"#);
    assert!(d.validate().unwrap_err().contains("root node"));
    // A source-first chain still validates.
    let ok = doc(r#"{ "name": "n", "root": { "type": "chain", "stages": [
                { "type": "sine", "freq": 440 }, { "type": "lowpass", "cutoff": 800 }
            ] } }"#);
    assert!(ok.validate().is_ok());
}

#[test]
fn validate_rejects_duplicate_automation_lanes() {
    // The renderer applies the first matching lane; a second is silently dead.
    let d = doc(
        r#"{ "name": "n", "version": 2, "root": { "type": "tracks", "tracks": [
                { "id": "a", "node": { "type": "noise" },
                  "automation": [
                    { "target": "gain", "points": [ { "t": 0, "v": 0.5 } ] },
                    { "target": "gain", "points": [ { "t": 0, "v": 1.0 } ] }
                  ] }
            ] } }"#,
    );
    assert!(
        d.validate()
            .unwrap_err()
            .contains("duplicate automation lane")
    );
}

#[test]
fn validate_rejects_silent_all_zero_envmod() {
    // The same flatten footgun as Node::Env: the "adsr" object is silently
    // dropped and the parameter would pin at `from`.
    let d = doc(r#"{ "name": "n", "root": { "type": "chain", "stages": [
                { "type": "noise" },
                { "type": "lowpass",
                  "cutoff": { "env": { "adsr": { "a": 0.1 }, "from": 200, "to": 800 } } }
            ] } }"#);
    assert!(d.validate().unwrap_err().contains("env is constant"));
    let ok = doc(r#"{ "name": "n", "root": { "type": "chain", "stages": [
                { "type": "noise" },
                { "type": "lowpass", "cutoff": { "env": { "a": 0.1, "from": 200, "to": 800 } } }
            ] } }"#);
    assert!(ok.validate().is_ok());
}

#[test]
fn validate_bounds_graph_depth() {
    // serde caps JSON nesting, but a programmatic document can nest without
    // bound — the recursive validator/renderer would overflow the stack.
    let mut node = Node::Noise {
        color: NoiseColor::White,
    };
    for _ in 0..300 {
        node = Node::Chain { stages: vec![node] };
    }
    let d = SoundDoc::new("deep", node);
    assert!(d.validate().unwrap_err().contains("deeper"));
}

#[test]
fn children_covers_every_nested_graph() {
    // The one traversal definition: every combinator variant yields its
    // children in document order, leaves yield none — so walkers can't
    // silently skip a nesting spot (the historical `duck` bug class).
    let duck: Node =
        serde_json::from_str(r#"{ "type": "duck", "trigger": { "type": "sine", "freq": 55 } }"#)
            .unwrap();
    assert_eq!(duck.children().count(), 1, "a duck yields its trigger");
    let mix: Node = serde_json::from_str(
        r#"{ "type": "mix", "inputs": [ { "type": "noise" }, { "type": "sine", "freq": 440 } ] }"#,
    )
    .unwrap();
    assert_eq!(mix.children().count(), 2);
    let tracks: Node = serde_json::from_str(
        r#"{ "type": "tracks", "tracks": [ { "node": { "type": "noise" } } ],
             "master": [ { "type": "lowpass", "cutoff": 800 } ] }"#,
    )
    .unwrap();
    assert_eq!(
        tracks.children().count(),
        2,
        "a tracks node yields its layers, then the master chain"
    );
    let leaf: Node = serde_json::from_str(r#"{ "type": "sine", "freq": 440 }"#).unwrap();
    assert_eq!(leaf.children().count(), 0);
}
