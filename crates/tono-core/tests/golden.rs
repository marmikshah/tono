//! Golden-corpus replay tests.
//!
//! Every byte-identity test elsewhere is *relative* (streaming vs offline, or
//! render(d) == render(d) in one process). These tests pin the exact rendered
//! bytes of representative documents against checked-in hashes, so a kernel
//! change that shifts BOTH paths together — the failure the engine-version
//! gate exists to prevent — fails CI instead of silently breaking every
//! existing user document.
//!
//! When a mismatch is *intentional* (a new engine revision changing only docs
//! that opt in), re-pin by running:
//! `cargo test -p tono-core --test golden -- --nocapture` and copying the
//! printed table.

use tono_core::dsl::{Adsr, SeqWave, SoundDoc};
use tono_core::render::render_product;
use tono_core::song::{Song, note};

/// FNV-1a over the little-endian bit patterns of the samples.
fn hash_signal(samples: &[f32]) -> u64 {
    let mut h: u64 = 0xCBF2_9CE4_8422_2325;
    for s in samples {
        for b in s.to_bits().to_le_bytes() {
            h ^= b as u64;
            h = h.wrapping_mul(0x0000_0100_0000_01B3);
        }
    }
    h
}

fn parse(json: &str) -> SoundDoc {
    let doc: SoundDoc = serde_json::from_str(json).expect("corpus doc parses");
    doc.validate().expect("corpus doc validates");
    doc
}

struct Case {
    name: &'static str,
    json: &'static str,
    mono: u64,
    stereo: Option<(u64, u64)>,
}

/// One document per kernel family × engine revision. Durations are short so
/// the whole corpus renders in well under a second.
const CORPUS: &[Case] = &[
    Case {
        name: "blip-legacy",
        json: r#"{ "name": "blip-legacy", "duration": 0.2, "root": { "type": "mul", "inputs": [
            { "type": "sine", "freq": 880 },
            { "type": "env", "a": 0.002, "d": 0.08, "s": 0.0, "r": 0.05 } ] } }"#,
        mono: 0xecdcebf2ca9d38c4,
        stereo: None,
    },
    Case {
        name: "noise-v0",
        json: r#"{ "name": "noise-v0", "duration": 0.15, "seed": 7,
            "root": { "type": "noise", "color": "pink" } }"#,
        mono: 0xa3f0e6e7c8d7e183,
        stereo: None,
    },
    Case {
        name: "noise-v2",
        json: r#"{ "name": "noise-v2", "duration": 0.15, "seed": 7, "engine": 2,
            "root": { "type": "noise", "color": "pink" } }"#,
        mono: 0xb73db8207746cdbf,
        stereo: None,
    },
    Case {
        name: "pwm-slide",
        json: r#"{ "name": "pwm-slide", "duration": 0.25, "engine": 2, "root": { "type": "square",
            "freq": { "slide": { "from": 220, "to": 440, "secs": 0.2, "curve": "exp" } },
            "duty": { "lfo": { "rate": 8, "depth": 0.3, "center": 0.5 } } } }"#,
        mono: 0x6ca3dc84607bbe3c,
        stereo: None,
    },
    Case {
        name: "fm-bell",
        json: r#"{ "name": "fm-bell", "duration": 0.3, "engine": 2, "root": { "type": "fm",
            "freq": 660, "ratio": 3.5,
            "index": { "slide": { "from": 6, "to": 0.5, "secs": 0.25 } } } }"#,
        mono: 0x9ee1ff99f21948fc,
        stereo: None,
    },
    Case {
        name: "supersaw-fx",
        json: r#"{ "name": "supersaw-fx", "duration": 0.3, "engine": 2, "root": { "type": "chain", "stages": [
            { "type": "super", "freq": 110, "voices": 7, "detune_cents": 25 },
            { "type": "lowpass", "cutoff": 1200, "q": 0.9 },
            { "type": "delay", "secs": 0.09, "feedback": 0.35 },
            { "type": "reverb", "room": 0.4, "mix": 0.25 } ] } }"#,
        mono: 0xb0f3fe096484eb4f,
        stereo: None,
    },
    Case {
        name: "dust-crackle",
        json: r#"{ "name": "dust-crackle", "duration": 0.25, "seed": 11, "engine": 2,
            "root": { "type": "chain", "stages": [
            { "type": "dust", "density": 220, "decay": 0.004 },
            { "type": "highpass", "cutoff": 1800, "q": 0.7 } ] } }"#,
        mono: 0xf983e1ee44a7124e,
        stereo: None,
    },
    Case {
        name: "seq-saw-groove",
        json: r#"{ "name": "seq-saw-groove", "duration": 1.0, "seed": 3, "engine": 2,
            "root": { "type": "seq", "bpm": 240, "wave": "sawtooth", "swing": 0.55, "humanize": 0.2,
            "env": { "a": 0.005, "d": 0.05, "s": 0.6, "r": 0.08 },
            "notes": [
                { "step": 0, "len": 2, "pitch": "C2" },
                { "step": 2, "len": 2, "pitch": "G2", "gain": 0.8 },
                { "step": 4, "len": 2, "pitch": "A#2" },
                { "step": 6, "len": 2, "pitch": "C3", "gain": 0.7 } ] } }"#,
        mono: 0x0667b70a01616ae1,
        stereo: None,
    },
    Case {
        name: "seq-piano-v3",
        json: r#"{ "name": "seq-piano-v3", "duration": 1.0, "engine": 3,
            "root": { "type": "seq", "bpm": 240, "wave": "piano",
            "env": { "a": 0.002, "s": 1.0, "r": 0.2 },
            "notes": [
                { "step": 0, "len": 4, "pitch": "C4" },
                { "step": 0, "len": 4, "pitch": "E4", "gain": 0.9 },
                { "step": 4, "len": 4, "pitch": "G3", "gain": 0.7 } ] } }"#,
        mono: 0x3f76e2c14bb45187,
        stereo: None,
    },
    Case {
        name: "seq-kit-808",
        json: r#"{ "name": "seq-kit-808", "duration": 1.0, "seed": 5, "engine": 3,
            "root": { "type": "seq", "bpm": 240, "wave": "kit", "kit": "808",
            "env": { "a": 0.001, "d": 0.1, "s": 0.5, "r": 0.1 },
            "notes": [
                { "step": 0, "len": 1, "pitch": "midi:36" },
                { "step": 2, "len": 1, "pitch": "midi:38", "gain": 0.9 },
                { "step": 4, "len": 1, "pitch": "midi:36" },
                { "step": 5, "len": 1, "pitch": "midi:42", "gain": 0.6 },
                { "step": 6, "len": 1, "pitch": "midi:38" } ] } }"#,
        mono: 0x7cc71e1213407ad3,
        stereo: None,
    },
    Case {
        name: "seq-bass-v3",
        json: r#"{ "name": "seq-bass-v3", "duration": 1.0, "engine": 3,
            "root": { "type": "seq", "bpm": 240, "wave": "bass",
            "env": { "a": 0.005, "d": 0.08, "s": 0.7, "r": 0.1 },
            "notes": [
                { "step": 0, "len": 3, "pitch": "E1" },
                { "step": 4, "len": 3, "pitch": "G1", "gain": 0.85 } ] } }"#,
        mono: 0x221a786a5641fc9d,
        stereo: None,
    },
    Case {
        // Pins the whole mixer path: pan law, automation lanes, master
        // reverb's decorrelated tails, per-channel normalize (the engine ≤ 3
        // behavior), and the final peak limit — mono mid plus both channels.
        name: "tracks-mix",
        json: r#"{ "name": "tracks-mix", "duration": 0.5, "seed": 9, "engine": 3,
            "normalize": { "target_lufs": -14, "ceiling_dbtp": -1.0 },
            "root": { "type": "tracks", "tracks": [
                { "id": "pad", "node": { "type": "sine", "freq": 220 }, "pan": -0.8, "gain": 0.3 },
                { "id": "hiss", "node": { "type": "noise", "color": "white" }, "pan": 0.9, "gain": 0.6,
                  "automation": [ { "target": "gain", "points": [
                      { "t": 0.0, "v": 0.1 }, { "t": 0.4, "v": 0.8 } ] } ] },
                { "id": "lead", "node": { "type": "square", "freq": 440, "duty": 0.25 }, "gain": 0.4, "at": 0.1 }
            ], "master": [ { "type": "reverb", "room": 0.3, "mix": 0.2 } ] } }"#,
        mono: 0xfbd248e0796957b7,
        stereo: Some((0xf7767b2a46f37bc2, 0x413e706e94cb48a9)),
    },
    Case {
        name: "loop-bed",
        json: r#"{ "name": "loop-bed", "duration": 0.6, "seed": 2, "engine": 2,
            "playback": { "mode": "loop", "start_secs": 0.1, "end_secs": 0.5, "crossfade_secs": 0.08 },
            "root": { "type": "chain", "stages": [
                { "type": "noise", "color": "brown" },
                { "type": "lowpass", "cutoff": 600, "q": 0.8 } ] } }"#,
        mono: 0x430d046059c90676,
        stereo: None,
    },
];

/// Song fluent path golden: the arrangement layer compiles through `to_doc`
/// into the same render path, pinned end to end.
fn fluent_song_doc() -> SoundDoc {
    let amp = Adsr {
        a: 0.005,
        d: 0.1,
        s: 0.8,
        r: 0.2,
        punch: 0.0,
    };
    let mut song = Song::new("golden-groove", 240.0);
    song.add_track("bass", SeqWave::Bass, amp);
    song.add_track("keys", SeqWave::Epiano, amp);
    song.add_pattern(
        "riff",
        1,
        vec![note(0, 4, "C2"), note(8, 4, "G2"), note(12, 4, "A#2")],
    );
    song.add_pattern("stab", 1, vec![note(4, 2, "C4"), note(6, 2, "D#4")]);
    song.arrange("bass", "riff", 0);
    song.arrange("keys", "stab", 0);
    song.to_doc().expect("song compiles")
}

const FLUENT_SONG_MONO: u64 = 0x2184e2154f5b92e6;
const FLUENT_SONG_LEFT: u64 = 0x2184e2154f5b92e6;
const FLUENT_SONG_RIGHT: u64 = 0x2184e2154f5b92e6;

fn check(name: &str, doc: &SoundDoc, mono: u64, stereo: Option<(u64, u64)>) -> Option<String> {
    let p = render_product(doc);
    let got_mono = hash_signal(&p.mono);
    let got_stereo = p
        .stereo
        .as_ref()
        .map(|(l, r)| (hash_signal(l), hash_signal(r)));
    if got_mono == mono && got_stereo == stereo {
        return None;
    }
    Some(match got_stereo {
        Some((l, r)) => {
            format!("{name}: mono: 0x{got_mono:016x}, stereo: Some((0x{l:016x}, 0x{r:016x}))")
        }
        None => format!("{name}: mono: 0x{got_mono:016x}, stereo: None"),
    })
}

#[test]
fn golden_corpus_replays_byte_identically() {
    let mut mismatches = Vec::new();
    for c in CORPUS {
        if let Some(m) = check(c.name, &parse(c.json), c.mono, c.stereo) {
            mismatches.push(m);
        }
    }
    if let Some(m) = check(
        "fluent-song",
        &fluent_song_doc(),
        FLUENT_SONG_MONO,
        Some((FLUENT_SONG_LEFT, FLUENT_SONG_RIGHT)),
    ) {
        mismatches.push(m);
    }
    assert!(
        mismatches.is_empty(),
        "golden renders changed — if intentional (an engine-gated upgrade), re-pin:\n{}",
        mismatches.join("\n")
    );
}

#[test]
fn serde_roundtrip_preserves_the_render() {
    for c in CORPUS {
        let doc = parse(c.json);
        let reparsed: SoundDoc =
            serde_json::from_str(&serde_json::to_string(&doc).expect("serialize"))
                .expect("reparse");
        let a = render_product(&doc);
        let b = render_product(&reparsed);
        let bits = |s: &[f32]| s.iter().map(|x| x.to_bits()).collect::<Vec<_>>();
        assert_eq!(
            bits(&a.mono),
            bits(&b.mono),
            "{}: serialize→reparse changed the render",
            c.name
        );
    }
}

#[test]
fn example_recipes_replay_byte_identically() {
    // CLAUDE.md promises the docs/examples recipes replay in CI — enforce it.
    let root = concat!(env!("CARGO_MANIFEST_DIR"), "/../../docs/examples/");
    for (file, expected) in [
        ("blip.json", 0x412f073451d5a804u64),
        ("hat.json", 0xbc3ab825679ceef7u64),
    ] {
        let json = std::fs::read_to_string(format!("{root}{file}"))
            .unwrap_or_else(|e| panic!("read {file}: {e}"));
        let doc = parse(&json);
        let got = hash_signal(&render_product(&doc).mono);
        assert_eq!(
            got, expected,
            "{file}: expected 0x{expected:016x}, got 0x{got:016x}"
        );
    }
}
