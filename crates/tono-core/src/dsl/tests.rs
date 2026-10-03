use super::*;
use crate::dsl::ValidateError;

fn roundtrip(json: &str) -> serde_json::Value {
    let doc: SoundDoc = serde_json::from_str(json).expect("deserialize");
    serde_json::to_value(&doc).expect("serialize")
}

fn doc_with_root(root: &str) -> SoundDoc {
    serde_json::from_str(&format!(
        r#"{{ "name": "t", "duration": 0.2, "engine": 5, "root": {root} }}"#
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
    assert_eq!(doc.version, SCHEMA_VERSION);
    assert_eq!(doc.engine, ENGINE_VERSION);
    assert!(matches!(doc.stereo, Stereo::Mono));
    assert!(matches!(doc.playback, Playback::OneShot));
}

#[test]
fn v2_tracks_reject_doc_level_stereo() {
    let doc: SoundDoc = serde_json::from_str(
        r#"{ "name": "band", "duration": 0.2, "version": 2,
                "stereo": { "mode": "wide" },
                "root": { "type": "tracks",
                  "tracks": [ { "id": "a", "node": { "type": "sine", "freq": 220 } } ] } }"#,
    )
    .unwrap();
    let err = doc.validate().unwrap_err();
    assert!(err.contains("per-layer pan"), "{err}");
}

#[test]
fn unsupported_schema_versions_are_rejected() {
    let mut doc = SoundDoc::new(
        "beep",
        Node::Sine {
            freq: Value::Const(440.0),
        },
    );
    assert_eq!(doc.validate(), Ok(()));
    for version in [0, 1, SCHEMA_VERSION + 1] {
        doc.version = version;
        assert!(
            doc.validate()
                .unwrap_err()
                .contains("unsupported document version")
        );
    }
}

#[test]
fn unsupported_engines_are_rejected() {
    let mut doc = SoundDoc::new(
        "beep",
        Node::Sine {
            freq: Value::Const(440.0),
        },
    );
    assert_eq!(doc.validate(), Ok(()));
    for engine in [0, 1, 2, 3, 4, ENGINE_VERSION + 1] {
        doc.engine = engine;
        assert!(doc.validate().unwrap_err().contains("unsupported engine"));
    }
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
fn tremolo_defaults_and_validates_its_bounds() {
    // `"type": "tremolo"` alone is valid: rate 6 Hz, depth 0.5.
    let d = doc_with_root(
        r#"{ "type": "chain", "stages": [
                { "type": "sine", "freq": 220 },
                { "type": "tremolo" } ] }"#,
    );
    assert_eq!(d.validate(), Ok(()));
    let Node::Chain { stages } = &d.root else {
        panic!("still a chain");
    };
    let Node::Tremolo { rate, depth } = &stages[1] else {
        panic!("still a tremolo");
    };
    assert_eq!(*rate, 6.0);
    assert_eq!(*depth, 0.5);

    let trem = |rate: &str, depth: &str| {
        doc_with_root(&format!(
            r#"{{ "type": "chain", "stages": [
                    {{ "type": "sine", "freq": 220 }},
                    {{ "type": "tremolo", "rate": {rate}, "depth": {depth} }} ] }}"#
        ))
        .validate()
    };
    assert!(trem("0.0", "1.0").is_ok());
    assert!(trem("40.0", "0.0").is_ok());
    assert!(trem("-1", "0.5").unwrap_err().contains("tremolo.rate"));
    assert!(trem("41", "0.5").unwrap_err().contains("tremolo.rate"));
    // 1e308 deserializes to f32 inf — the finite check fires first.
    assert!(trem("1e308", "0.5").unwrap_err().contains("tremolo.rate"));
    assert!(trem("6", "1.5").unwrap_err().contains("tremolo.depth"));
    assert!(trem("6", "-0.1").unwrap_err().contains("tremolo.depth"));
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

#[test]
fn wavetable_defaults_and_validates_its_bounds() {
    // A bare node is valid: `wave` defaults to basic, `position` to 0.
    let d = doc_with_root(r#"{ "type": "wavetable", "freq": 220 }"#);
    assert_eq!(d.validate(), Ok(()));
    let Node::Wavetable { wave, position, .. } = &d.root else {
        panic!("still a wavetable");
    };
    assert_eq!(*wave, WavetableKind::Basic);
    assert!(matches!(position, Value::Const(c) if *c == 0.0));

    // A constant morph position outside 0..1 is rejected...
    assert!(
        doc_with_root(r#"{ "type": "wavetable", "freq": 220, "position": 1.5 }"#)
            .validate()
            .unwrap_err()
            .contains("wavetable.position")
    );
    // ...while a modulated one clamps at render time (same policy as duty).
    assert!(
        doc_with_root(
            r#"{ "type": "wavetable", "freq": 220,
                 "position": { "lfo": { "rate": 1, "depth": 0.5, "center": 0.5 } } }"#
        )
        .validate()
        .is_ok()
    );
}

#[test]
fn validate_rejects_bad_sidechain_wiring() {
    // Unknown source id.
    let d = doc(
        r#"{ "name": "n", "version": 2, "root": { "type": "tracks", "tracks": [
                { "id": "pad", "node": { "type": "sine", "freq": 220 },
                  "sidechain": { "source": "kick" } }
            ] } }"#,
    );
    let err = d.validate().unwrap_err();
    assert!(err.contains("'kick'"), "{err}");
    assert!(err.contains("not a layer id"), "{err}");
    // A track cannot duck to its own signal.
    let d = doc(
        r#"{ "name": "n", "version": 2, "root": { "type": "tracks", "tracks": [
                { "id": "kick", "node": { "type": "noise" },
                  "sidechain": { "source": "kick" } }
            ] } }"#,
    );
    let err = d.validate().unwrap_err();
    assert!(err.contains("the track itself"), "{err}");
    // Follower-of-follower chains are rejected (this also rejects 2-cycles:
    // a→b with b→a fails here for whichever follower is checked first).
    let d = doc(
        r#"{ "name": "n", "version": 2, "root": { "type": "tracks", "tracks": [
                { "id": "kick", "node": { "type": "noise" } },
                { "id": "bass", "node": { "type": "sine", "freq": 55 },
                  "sidechain": { "source": "kick" } },
                { "id": "pad", "node": { "type": "sine", "freq": 220 },
                  "sidechain": { "source": "bass" } }
            ] } }"#,
    );
    let err = d.validate().unwrap_err();
    assert!(err.contains("follower-of-follower"), "{err}");
    // Out-of-range sidechain params.
    let d = doc(
        r#"{ "name": "n", "version": 2, "root": { "type": "tracks", "tracks": [
                { "id": "kick", "node": { "type": "noise" } },
                { "id": "pad", "node": { "type": "sine", "freq": 220 },
                  "sidechain": { "source": "kick", "amount": 1.5 } }
            ] } }"#,
    );
    assert!(d.validate().unwrap_err().contains("sidechain.amount"));
    // A sane link — including several tracks following the same source —
    // validates.
    let ok = doc(
        r#"{ "name": "n", "version": 2, "root": { "type": "tracks", "tracks": [
                { "id": "pad", "node": { "type": "sine", "freq": 220 },
                  "sidechain": { "source": "kick" } },
                { "id": "bass", "node": { "type": "sine", "freq": 55 },
                  "sidechain": { "source": "kick" } },
                { "id": "kick", "node": { "type": "noise" } }
            ] } }"#,
    );
    assert_eq!(ok.validate(), Ok(()));
}

#[test]
fn validate_rejects_bad_bus_wiring() {
    let mk = |buses: &str, tracks: &str| {
        doc(&format!(
            r#"{{ "name":"t", "duration":1.0, "version":2, "root":{{ "type":"tracks",
                "buses":{buses}, "tracks":{tracks} }} }}"#
        ))
    };
    let good = mk(
        r#"[ { "id":"verb", "gain":0.8, "effects":[ { "type":"reverb", "room":0.5 } ] } ]"#,
        r#"[ { "id":"a", "node":{ "type":"sine", "freq":440 }, "bus":"verb",
              "sends":[ { "bus":"verb", "amount":0.4 } ] } ]"#,
    );
    assert!(good.validate().is_ok(), "{:?}", good.validate());
    // Unknown bus on a route, unknown bus / bad amount / duplicate on sends.
    for tracks in [
        r#"[ { "id":"a", "node":{ "type":"sine", "freq":440 }, "bus":"ghost" } ]"#,
        r#"[ { "id":"a", "node":{ "type":"sine", "freq":440 },
              "sends":[ { "bus":"ghost", "amount":0.5 } ] } ]"#,
        r#"[ { "id":"a", "node":{ "type":"sine", "freq":440 },
              "sends":[ { "bus":"verb", "amount":1.5 } ] } ]"#,
        r#"[ { "id":"a", "node":{ "type":"sine", "freq":440 },
              "sends":[ { "bus":"verb", "amount":0.3 }, { "bus":"verb", "amount":0.2 } ] } ]"#,
    ] {
        let d = mk(
            r#"[ { "id":"verb", "effects":[ { "type":"reverb", "room":0.5 } ] } ]"#,
            tracks,
        );
        assert!(d.validate().is_err(), "{tracks} should be rejected");
    }
    // Duplicate bus ids, a bus/track name collision, a source on the insert
    // chain, and 'master' as a bus id are all rejected.
    for (buses, why) in [
        (
            r#"[ { "id":"verb" }, { "id":"verb" } ]"#,
            "duplicate bus id",
        ),
        (r#"[ { "id":"a" } ]"#, "bus id collides with a layer id"),
        (
            r#"[ { "id":"verb", "effects":[ { "type":"sine", "freq":440 } ] } ]"#,
            "a source on the insert chain",
        ),
        (r#"[ { "id":"master" } ]"#, "'master' is reserved"),
    ] {
        let d = mk(
            buses,
            r#"[ { "id":"a", "node":{ "type":"sine", "freq":440 } } ]"#,
        );
        assert!(d.validate().is_err(), "{why}: should be rejected");
    }
}

#[test]
fn validate_bounds_convolve_params() {
    let mk = |extra: &str| {
        doc(&format!(
            r#"{{ "name": "n", "root": {{ "type": "chain", "stages": [
                {{ "type": "noise" }}, {{ "type": "convolve", {extra} }}
            ] }} }}"#
        ))
    };
    assert!(mk(r#""decay": 1.5"#).validate().is_ok());
    // size 0 = follow decay (the serde default).
    assert!(mk(r#""size": 0.0, "predelay": 0.1"#).validate().is_ok());
    // Unbounded IR times would let a validated doc request arbitrary
    // allocations (the delay.secs pattern).
    assert!(
        mk(r#""decay": 31.0"#)
            .validate()
            .unwrap_err()
            .contains("convolve.decay")
    );
    assert!(
        mk(r#""decay": 0.0"#)
            .validate()
            .unwrap_err()
            .contains("convolve.decay")
    );
    assert!(
        mk(r#""size": 1e9"#)
            .validate()
            .unwrap_err()
            .contains("convolve.size")
    );
    assert!(
        mk(r#""predelay": -0.1"#)
            .validate()
            .unwrap_err()
            .contains("convolve.predelay")
    );
    assert!(
        mk(r#""predelay": 1e308"#)
            .validate()
            .unwrap_err()
            .contains("convolve.predelay")
    );
    assert!(
        mk(r#""damp": 1.5"#)
            .validate()
            .unwrap_err()
            .contains("convolve.damp")
    );
    assert!(
        mk(r#""mix": -0.1"#)
            .validate()
            .unwrap_err()
            .contains("convolve.mix")
    );
}

#[test]
fn validate_bounds_granular_params() {
    let mk = |extra: &str| {
        doc(&format!(
            r#"{{ "name": "n", "root": {{ "type": "chain", "stages": [
                {{ "type": "sine", "freq": 220 }}, {{ "type": "granular", {extra} }}
            ] }} }}"#
        ))
    };
    assert!(
        mk(r#""grain_ms": 80, "density": 25, "pitch": 1.0"#)
            .validate()
            .is_ok()
    );
    assert!(
        mk(r#""grain_ms": 4.0"#)
            .validate()
            .unwrap_err()
            .contains("granular.grain_ms")
    );
    assert!(
        mk(r#""grain_ms": 501.0"#)
            .validate()
            .unwrap_err()
            .contains("granular.grain_ms")
    );
    assert!(
        mk(r#""density": 0.05"#)
            .validate()
            .unwrap_err()
            .contains("granular.density")
    );
    assert!(
        mk(r#""density": 1e308"#)
            .validate()
            .unwrap_err()
            .contains("granular.density")
    );
    assert!(
        mk(r#""pitch": 0.2"#)
            .validate()
            .unwrap_err()
            .contains("granular.pitch")
    );
    assert!(
        mk(r#""pitch": 5.0"#)
            .validate()
            .unwrap_err()
            .contains("granular.pitch")
    );
    assert!(
        mk(r#""spread": 2.0"#)
            .validate()
            .unwrap_err()
            .contains("granular.spread")
    );
    assert!(
        mk(r#""mix": 1.1"#)
            .validate()
            .unwrap_err()
            .contains("granular.mix")
    );
}

#[test]
fn convolve_and_granular_defaults_deserialize() {
    // f32 → JSON round-trips through the f32's shortest form (0.3f32 prints as
    // 0.30000001192092896), so compare as f32, not against f64 literals.
    let num = |v: &serde_json::Value| v.as_f64().unwrap() as f32;
    let c = doc(r#"{ "name": "n", "root": { "type": "chain", "stages": [
                { "type": "impact" }, { "type": "convolve" } ] } }"#);
    assert!(c.validate().is_ok());
    let v = serde_json::to_value(&c).unwrap();
    let cv = &v["root"]["stages"][1];
    assert_eq!(num(&cv["decay"]), 1.5);
    assert_eq!(num(&cv["size"]), 0.0); // 0 = follow decay
    assert_eq!(num(&cv["predelay"]), 0.0);
    assert_eq!(num(&cv["damp"]), 0.3);
    assert_eq!(num(&cv["mix"]), 0.35);
    let g = doc(r#"{ "name": "n", "root": { "type": "chain", "stages": [
                { "type": "sine", "freq": 220 }, { "type": "granular" } ] } }"#);
    assert!(g.validate().is_ok());
    let v = serde_json::to_value(&g).unwrap();
    let gr = &v["root"]["stages"][1];
    assert_eq!(num(&gr["grain_ms"]), 80.0);
    assert_eq!(num(&gr["density"]), 25.0);
    assert_eq!(num(&gr["pitch"]), 1.0);
    assert_eq!(num(&gr["spread"]), 0.3);
    assert_eq!(num(&gr["mix"]), 0.5);
}
