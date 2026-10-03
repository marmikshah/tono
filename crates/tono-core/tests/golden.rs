//! Current-engine PCM regression corpus.
//!
//! These shared pins were captured from the unchanged engine-5 implementation
//! before removing historical kernels. They cover sound, effects, loop output,
//! normalization, groove, automation, buses and stereo mixing on every target.
//! Change a pin only after investigating an intentional product change.

use tono_core::dsl::{Adsr, SeqWave, SoundDoc};
use tono_core::render::render_product;
use tono_core::song::{Song, note};

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

type Pins = (u64, Option<(u64, u64)>);

struct Case {
    name: &'static str,
    json: &'static str,
    pins: Pins,
}

const CORPUS: &[Case] = &[
    Case {
        name: "blip",
        json: r#"{ "version": 2, "engine": 5, "name": "blip", "duration": 0.2, "root": { "type": "mul", "inputs": [
            { "type": "sine", "freq": 880 },
            { "type": "env", "a": 0.002, "d": 0.08, "s": 0.0, "r": 0.05 } ] } }"#,
        pins: (0xd1ffb6037f37edd6, None),
    },
    Case {
        name: "noise",
        json: r#"{ "version": 2, "engine": 5, "name": "noise", "duration": 0.15, "seed": 7,
            "root": { "type": "noise", "color": "pink" } }"#,
        pins: (0xb73db8207746cdbf, None),
    },
    Case {
        name: "pwm-slide",
        json: r#"{ "version": 2, "name": "pwm-slide", "duration": 0.25, "engine": 5, "root": { "type": "square",
            "freq": { "slide": { "from": 220, "to": 440, "secs": 0.2, "curve": "exp" } },
            "duty": { "lfo": { "rate": 8, "depth": 0.3, "center": 0.5 } } } }"#,
        pins: (0x607a88e688b120a8, None),
    },
    Case {
        name: "fm-bell",
        json: r#"{ "version": 2, "name": "fm-bell", "duration": 0.3, "engine": 5, "root": { "type": "fm",
            "freq": 660, "ratio": 3.5,
            "index": { "slide": { "from": 6, "to": 0.5, "secs": 0.25 } } } }"#,
        pins: (0x3325c108bf2a0572, None),
    },
    Case {
        name: "supersaw-fx",
        json: r#"{ "version": 2, "name": "supersaw-fx", "duration": 0.3, "engine": 5, "root": { "type": "chain", "stages": [
            { "type": "super", "freq": 110, "voices": 7, "detune_cents": 25 },
            { "type": "lowpass", "cutoff": 1200, "q": 0.9 },
            { "type": "delay", "secs": 0.09, "feedback": 0.35 },
            { "type": "reverb", "room": 0.4, "mix": 0.25 } ] } }"#,
        pins: (0xb0f3fe096484eb4f, None),
    },
    Case {
        name: "dust-crackle",
        json: r#"{ "version": 2, "name": "dust-crackle", "duration": 0.25, "seed": 11, "engine": 5,
            "root": { "type": "chain", "stages": [
            { "type": "dust", "density": 220, "decay": 0.004 },
            { "type": "highpass", "cutoff": 1800, "q": 0.7 } ] } }"#,
        pins: (0xf983e1ee44a7124e, None),
    },
    Case {
        name: "seq-saw-groove",
        json: r#"{ "version": 2, "name": "seq-saw-groove", "duration": 1.0, "seed": 3, "engine": 5,
            "root": { "type": "seq", "bpm": 240, "wave": "sawtooth", "swing": 0.55, "humanize": 0.2,
            "env": { "a": 0.005, "d": 0.05, "s": 0.6, "r": 0.08 },
            "notes": [
                { "step": 0, "len": 2, "pitch": "C2" },
                { "step": 2, "len": 2, "pitch": "G2", "gain": 0.8 },
                { "step": 4, "len": 2, "pitch": "A#2" },
                { "step": 6, "len": 2, "pitch": "C3", "gain": 0.7 } ] } }"#,
        pins: (0x50c0d5745cbc52c8, None),
    },
    Case {
        name: "seq-piano",
        json: r#"{ "version": 2, "name": "seq-piano", "duration": 1.0, "engine": 5,
            "root": { "type": "seq", "bpm": 240, "wave": "piano",
            "env": { "a": 0.002, "s": 1.0, "r": 0.2 },
            "notes": [
                { "step": 0, "len": 4, "pitch": "C4" },
                { "step": 0, "len": 4, "pitch": "E4", "gain": 0.9 },
                { "step": 4, "len": 4, "pitch": "G3", "gain": 0.7 } ] } }"#,
        pins: (0xba0f80d266928aa5, None),
    },
    Case {
        name: "seq-kit-808",
        json: r#"{ "version": 2, "name": "seq-kit-808", "duration": 1.0, "seed": 5, "engine": 5,
            "root": { "type": "seq", "bpm": 240, "wave": "kit", "kit": "808",
            "env": { "a": 0.001, "d": 0.1, "s": 0.5, "r": 0.1 },
            "notes": [
                { "step": 0, "len": 1, "pitch": "midi:36" },
                { "step": 2, "len": 1, "pitch": "midi:38", "gain": 0.9 },
                { "step": 4, "len": 1, "pitch": "midi:36" },
                { "step": 5, "len": 1, "pitch": "midi:42", "gain": 0.6 },
                { "step": 6, "len": 1, "pitch": "midi:38" } ] } }"#,
        pins: (0xc705643819112094, None),
    },
    Case {
        name: "seq-bass",
        json: r#"{ "version": 2, "name": "seq-bass", "duration": 1.0, "engine": 5,
            "root": { "type": "seq", "bpm": 240, "wave": "bass",
            "env": { "a": 0.005, "d": 0.08, "s": 0.7, "r": 0.1 },
            "notes": [
                { "step": 0, "len": 3, "pitch": "E1" },
                { "step": 4, "len": 3, "pitch": "G1", "gain": 0.85 } ] } }"#,
        pins: (0xd63c37b27051b833, None),
    },
    Case {
        name: "tracks-mix",
        json: r#"{ "version": 2, "name": "tracks-mix", "duration": 0.5, "seed": 9, "engine": 5,
            "normalize": { "target_lufs": -14, "ceiling_dbtp": -1.0 },
            "root": { "type": "tracks", "tracks": [
                { "id": "pad", "node": { "type": "sine", "freq": 220 }, "pan": -0.8, "gain": 0.3 },
                { "id": "hiss", "node": { "type": "noise", "color": "white" }, "pan": 0.9, "gain": 0.6,
                  "automation": [ { "target": "gain", "points": [
                      { "t": 0.0, "v": 0.1 }, { "t": 0.4, "v": 0.8 } ] } ] },
                { "id": "lead", "node": { "type": "square", "freq": 440, "duty": 0.25 }, "gain": 0.4, "at": 0.1 }
            ], "master": [ { "type": "reverb", "room": 0.3, "mix": 0.2 } ] } }"#,
        pins: (
            0x5530dd4b12db56d3,
            Some((0xd9ab1d312795de78, 0x2538315d5e68a7d8)),
        ),
    },
    Case {
        name: "loop-bed",
        json: r#"{ "version": 2, "name": "loop-bed", "duration": 0.6, "seed": 2, "engine": 5,
            "playback": { "mode": "loop", "start_secs": 0.1, "end_secs": 0.5, "crossfade_secs": 0.08 },
            "root": { "type": "chain", "stages": [
                { "type": "noise", "color": "brown" },
                { "type": "lowpass", "cutoff": 600, "q": 0.8 } ] } }"#,
        pins: (0x54a2e8256379836e, None),
    },
    Case {
        name: "normalize-mono",
        json: r#"{ "version": 2, "name": "normalize-mono", "duration": 0.5, "engine": 5,
            "normalize": { "target_lufs": -16, "ceiling_dbtp": -1.0 },
            "root": { "type": "chain", "stages": [
                { "type": "sine", "freq": 440 }, { "type": "gain", "amount": 0.05 } ] } }"#,
        pins: (0x89befc4020ed7df2, None),
    },
    Case {
        name: "wavetable-basic",
        json: r#"{ "name": "wavetable-basic", "duration": 0.25, "version": 2, "engine": 5,
            "root": { "type": "wavetable", "wave": "basic", "freq": 220, "position": 0.5 } }"#,
        pins: (0x898b1435864fab55, None),
    },
    Case {
        name: "wavetable-morph",
        json: r#"{ "name": "wavetable-morph", "duration": 0.3, "version": 2, "engine": 5,
            "root": { "type": "wavetable", "wave": "harmonics", "freq": 110,
                "position": { "lfo": { "rate": 2, "depth": 0.5, "center": 0.5 } } } }"#,
        pins: (0x09d7752a7fde544d, None),
    },
    Case {
        name: "tremolo-wobble",
        json: r#"{ "name": "tremolo-wobble", "duration": 0.3, "version": 2, "engine": 5,
            "root": { "type": "chain", "stages": [
                { "type": "sine", "freq": 440 },
                { "type": "tremolo", "rate": 6, "depth": 0.8 } ] } }"#,
        pins: (0x7659f9d636e19dcf, None),
    },
    Case {
        name: "convolve-space",
        json: r#"{ "name": "convolve-space", "duration": 0.5, "seed": 4, "version": 2, "engine": 5,
            "root": { "type": "chain", "stages": [
                { "type": "mul", "inputs": [
                    { "type": "noise", "color": "white" },
                    { "type": "env", "a": 0.001, "d": 0.02, "s": 0.0, "r": 0.01 } ] },
                { "type": "convolve", "decay": 0.25, "predelay": 0.01, "damp": 0.5, "mix": 0.45 } ] } }"#,
        pins: (0x870954514442334e, None),
    },
    Case {
        name: "granular-cloud",
        json: r#"{ "name": "granular-cloud", "duration": 0.4, "seed": 8, "version": 2, "engine": 5,
            "root": { "type": "chain", "stages": [
                { "type": "sine", "freq": 220 },
                { "type": "granular", "grain_ms": 40, "density": 25, "pitch": 2.0,
                    "spread": 0.3, "mix": 0.7 } ] } }"#,
        pins: (0xc67f6bf629bd6010, None),
    },
    Case {
        name: "tracks-sidechain",
        json: r#"{ "name": "tracks-sidechain", "duration": 1.0, "seed": 6, "version": 2, "engine": 5,
            "root": { "type": "tracks", "tracks": [
                { "id": "kick", "node": { "type": "seq", "bpm": 240, "wave": "kit", "kit": "808",
                    "env": { "a": 0.001, "d": 0.1, "s": 0.5, "r": 0.1 },
                    "notes": [
                        { "step": 0, "len": 1, "pitch": "midi:36" },
                        { "step": 4, "len": 1, "pitch": "midi:36" },
                        { "step": 8, "len": 1, "pitch": "midi:36" },
                        { "step": 12, "len": 1, "pitch": "midi:36" } ] } },
                { "id": "bass", "node": { "type": "seq", "bpm": 240, "wave": "sawtooth",
                    "env": { "a": 0.005, "d": 0.05, "s": 0.8, "r": 0.05 },
                    "notes": [ { "step": 0, "len": 16, "pitch": "C2" } ] },
                  "sidechain": { "source": "kick", "amount": 0.8, "attack": 0.005, "release": 0.15 } }
            ] } }"#,
        pins: (
            0x57513c995ccaae4b,
            Some((0x57513c995ccaae4b, 0x57513c995ccaae4b)),
        ),
    },
    Case {
        name: "seq-brass",
        json: r#"{ "name": "seq-brass", "duration": 0.8, "version": 2, "engine": 5,
            "root": { "type": "seq", "bpm": 240, "wave": "brass",
                "env": { "a": 0.01, "d": 0.05, "s": 1.0, "r": 0.1 },
                "notes": [
                    { "step": 0, "len": 4, "pitch": "C3" },
                    { "step": 4, "len": 4, "pitch": "E3" },
                    { "step": 8, "len": 6, "pitch": "G3", "gain": 0.85 } ] } }"#,
        pins: (0x22a2a149705a8c57, None),
    },
    Case {
        name: "seq-flute",
        json: r#"{ "name": "seq-flute", "duration": 0.8, "version": 2, "engine": 5,
            "root": { "type": "seq", "bpm": 240, "wave": "flute",
                "env": { "a": 0.02, "d": 0.05, "s": 1.0, "r": 0.15 },
                "notes": [
                    { "step": 0, "len": 8, "pitch": "A4" },
                    { "step": 8, "len": 6, "pitch": "C5", "gain": 0.9 } ] } }"#,
        pins: (0x5a80b5e2ed23c4e9, None),
    },
    Case {
        name: "seq-mallet",
        json: r#"{ "name": "seq-mallet", "duration": 0.8, "version": 2, "engine": 5,
            "root": { "type": "seq", "bpm": 240, "wave": "mallet",
                "env": { "a": 0.001, "d": 0.2, "s": 0.3, "r": 0.1 },
                "notes": [
                    { "step": 0, "len": 2, "pitch": "C5" },
                    { "step": 4, "len": 2, "pitch": "E5" },
                    { "step": 8, "len": 2, "pitch": "G5" },
                    { "step": 12, "len": 4, "pitch": "C6", "gain": 0.85 } ] } }"#,
        pins: (0x99248539bf8c4a76, None),
    },
    Case {
        name: "seq-bell",
        json: r#"{ "name": "seq-bell", "duration": 1.0, "version": 2, "engine": 5,
            "root": { "type": "seq", "bpm": 240, "wave": "bell",
                "env": { "a": 0.001, "d": 0.1, "s": 1.0, "r": 0.3 },
                "notes": [
                    { "step": 0, "len": 8, "pitch": "C4" },
                    { "step": 8, "len": 8, "pitch": "G4", "gain": 0.8 } ] } }"#,
        pins: (0xd6836b453e4da9e0, None),
    },
    Case {
        name: "seq-tempo-map",
        json: r#"{ "name": "seq-tempo-map", "duration": 2.5, "version": 2, "engine": 5,
            "root": { "type": "seq", "bpm": 120, "wave": "square",
                "tempo_map": [ { "at": { "num": 0, "den": 1 }, "bpm": 120 },
                               { "at": { "num": 4, "den": 1 }, "bpm": 90 } ],
                "env": { "a": 0.005, "d": 0.05, "s": 0.6, "r": 0.08 },
                "notes": [
                    { "step": 0, "len": 2, "pitch": "C4" },
                    { "step": 8, "len": 2, "pitch": "E4" },
                    { "step": 16, "len": 2, "pitch": "G4" },
                    { "step": 24, "len": 4, "pitch": "C5", "gain": 0.8 } ] } }"#,
        pins: (0x93bbbd9fe2057929, None),
    },
    Case {
        name: "tracks-automation-curves",
        json: r#"{ "name": "tracks-automation-curves", "duration": 1.0, "seed": 9, "version": 2, "engine": 5,
            "root": { "type": "tracks", "tracks": [
                { "id": "pad", "node": { "type": "sawtooth", "freq": 220 }, "gain": 0.5,
                  "automation": [
                    { "target": "gain", "curve": "exp", "points": [
                        { "t": 0.0, "v": 0.2 }, { "t": 0.6, "v": 0.9 } ] },
                    { "target": "pan", "curve": "step", "points": [
                        { "t": 0.0, "v": -0.5 }, { "t": 0.5, "v": 0.5 } ] }
                  ] }
            ] } }"#,
        pins: (
            0xa44af9c41a3020ba,
            Some((0x57ae3e432ec9b944, 0x12961b8a73918eb0)),
        ),
    },
    Case {
        name: "tracks-bus-mix",
        json: r#"{ "name": "tracks-bus-mix", "duration": 1.0, "seed": 6, "version": 2, "engine": 5,
            "root": { "type": "tracks",
                "buses": [ { "id": "verb", "gain": 0.8, "effects": [
                    { "type": "reverb", "room": 0.5, "mix": 0.6 } ] } ],
                "tracks": [
                    { "id": "kick", "node": { "type": "seq", "bpm": 240, "wave": "kit", "kit": "808",
                        "env": { "a": 0.001, "d": 0.1, "s": 0.5, "r": 0.1 },
                        "notes": [ { "step": 0, "len": 1, "pitch": "midi:36" } ] },
                      "sends": [ { "bus": "verb", "amount": 0.3 } ] },
                    { "id": "pad", "node": { "type": "sawtooth", "freq": 110 }, "gain": 0.4,
                      "pan": 0.3, "bus": "verb" }
                ] } }"#,
        pins: (
            0x5eb9cb07aca19a32,
            Some((0xa954f7a749bcda14, 0x2ddc608c754dc89e)),
        ),
    },
    Case {
        name: "kit-groove",
        json: r#"{ "version": 2, "name": "kit-groove", "duration": 0.5, "seed": 5, "engine": 5,
            "root": { "type": "seq", "bpm": 240, "wave": "kit", "kit": "808",
                "env": { "a": 0.001, "d": 0.1, "s": 0.5, "r": 0.1 },
                "notes": [
                    { "step": 0, "len": 1, "pitch": "midi:36" },
                    { "step": 2, "len": 1, "pitch": "midi:38", "gain": 0.9 },
                    { "step": 4, "len": 1, "pitch": "midi:42", "gain": 0.6 },
                    { "step": 6, "len": 1, "pitch": "midi:46", "gain": 0.7 } ] } }"#,
        pins: (0xaa7274eaf29a3da9, None),
    },
    Case {
        name: "drive-normalize",
        json: r#"{ "version": 2, "name": "drive-normalize", "duration": 0.4, "engine": 5,
            "normalize": { "target_lufs": -16, "ceiling_dbtp": -1.0 },
            "root": { "type": "chain", "stages": [
                { "type": "sawtooth", "freq": 110 },
                { "type": "drive", "amount": 0.6, "shape": "tanh" } ] } }"#,
        pins: (0x63a011a7a2370c5a, None),
    },
    Case {
        name: "tracks-composition",
        json: r#"{ "version": 2, "name": "tracks-composition", "duration": 1.0, "seed": 6, "engine": 5,
            "root": { "type": "tracks",
                "buses": [ { "id": "verb", "gain": 0.8, "effects": [
                    { "type": "reverb", "room": 0.5, "mix": 0.5 } ] } ],
                "tracks": [
                    { "id": "kick", "node": { "type": "seq", "bpm": 120, "wave": "kit", "kit": "808",
                        "tempo_map": [ { "at": { "num": 0, "den": 1 }, "bpm": 120 },
                                       { "at": { "num": 2, "den": 1 }, "bpm": 132 } ],
                        "env": { "a": 0.001, "d": 0.1, "s": 0.5, "r": 0.1 },
                        "notes": [ { "step": 0, "len": 1, "pitch": "midi:36" },
                                   { "step": 4, "len": 1, "pitch": "midi:36" },
                                   { "step": 8, "len": 1, "pitch": "midi:36" } ] },
                      "sends": [ { "bus": "verb", "amount": 0.3 } ] },
                    { "id": "bass", "node": { "type": "seq", "bpm": 120, "wave": "sawtooth",
                        "tempo_map": [ { "at": { "num": 0, "den": 1 }, "bpm": 120 },
                                       { "at": { "num": 2, "den": 1 }, "bpm": 132 } ],
                        "env": { "a": 0.005, "d": 0.05, "s": 0.8, "r": 0.05 },
                        "notes": [ { "step": 0, "len": 12, "pitch": "C2" } ] },
                      "sidechain": { "source": "kick", "amount": 0.8, "attack": 0.005, "release": 0.15 },
                      "automation": [ { "target": "gain", "curve": "exp", "points": [
                          { "t": 0.0, "v": 0.6 }, { "t": 0.5, "v": 0.9 } ] } ] },
                    { "id": "pad", "node": { "type": "sine", "freq": 220 }, "gain": 0.3,
                      "pan": 0.4, "bus": "verb" }
                ] } }"#,
        pins: (
            0x02fa7bbcf523b351,
            Some((0x93b3e8aad2483b3b, 0x88a1927a2dd38307)),
        ),
    },
];

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

fn hashes(doc: &SoundDoc) -> Pins {
    let product = render_product(doc);
    (
        hash_signal(&product.mono),
        product
            .stereo
            .as_ref()
            .map(|(l, r)| (hash_signal(l), hash_signal(r))),
    )
}

#[test]
fn golden_corpus_replays_byte_identically() {
    for c in CORPUS {
        assert_eq!(
            hashes(&parse(c.json)),
            c.pins,
            "{} changed from the baseline",
            c.name
        );
    }
    assert_eq!(
        hashes(&fluent_song_doc()),
        (
            0x79300bb0bdc42518,
            Some((0x79300bb0bdc42518, 0x79300bb0bdc42518))
        ),
        "fluent song changed from the baseline"
    );
}

#[test]
fn serde_roundtrip_preserves_the_render() {
    for c in CORPUS {
        let doc = parse(c.json);
        let reparsed = parse(&serde_json::to_string(&doc).unwrap());
        assert_eq!(hashes(&doc), hashes(&reparsed), "{} roundtrip", c.name);
    }
}

#[test]
fn example_recipes_replay_byte_identically() {
    let root = concat!(env!("CARGO_MANIFEST_DIR"), "/../../docs/examples/");
    for (file, pin) in [
        ("blip.json", 0x0af0b6df1a24ec76u64),
        ("hat.json", 0xbc3ab825679ceef7u64),
    ] {
        let json = std::fs::read_to_string(format!("{root}{file}")).unwrap();
        assert_eq!(
            hashes(&parse(&json)).0,
            pin,
            "{file} changed from the baseline"
        );
    }
}
