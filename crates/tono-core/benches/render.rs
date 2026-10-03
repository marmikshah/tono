//! Criterion benchmarks for the render hot path.
//!
//! The golden corpus pins every render byte-for-byte; these benches pin its
//! *speed* envelope, so a perf regression hiding inside a correct refactor
//! shows up in the report. One bench per representative kernel family —
//! oscillator+env, the stereo mixer with automation and a master reverb, a
//! melodic (piano) seq, a heavy effects chain, block-by-block streaming, the
//! song→program compiler, the scheduled runtime fill, and 8-track stem
//! mixing.
//!
//! Documents are short (0.3–0.5 s) so the whole suite runs in a couple of
//! minutes, and each doc is built ONCE outside the measured closure. Run with
//! `cargo bench -p tono-core` (report-only — not part of the test gate).

use criterion::{BatchSize, Criterion, criterion_group, criterion_main};
use std::hint::black_box;
use std::sync::Arc;
use tono_core::dsl::{Adsr, Bus, SeqWave, SoundDoc};
use tono_core::render;
use tono_core::runtime::{At, AudioSource, Command, Performance};
use tono_core::song::{CompileOptions, Song, note};
use tono_core::streaming::StreamGraph;

/// Parse + validate a bench document once (setup, never measured).
fn doc(json: &str) -> SoundDoc {
    let doc: SoundDoc = serde_json::from_str(json).expect("bench doc parses");
    doc.validate().expect("bench doc validates");
    doc
}

/// (a) The minimal SFX kernel: one oscillator under an ADSR.
fn blip() -> SoundDoc {
    doc(r#"{ "name": "blip", "duration": 0.3, "engine": 5,
            "root": { "type": "mul", "inputs": [
                { "type": "sine", "freq": 880 },
                { "type": "env", "a": 0.002, "d": 0.08, "s": 0.0, "r": 0.05 } ] } }"#)
}

/// (b) The stereo mixer: pan law, an automation lane, and a master reverb
/// with decorrelated tails (modeled on the golden `tracks-mix` case).
fn tracks_mix() -> SoundDoc {
    doc(
        r#"{ "name": "tracks-mix", "duration": 0.5, "seed": 9, "engine": 5,
            "normalize": { "target_lufs": -14, "ceiling_dbtp": -1.0 },
            "root": { "type": "tracks", "tracks": [
                { "id": "pad", "node": { "type": "sine", "freq": 220 }, "pan": -0.8, "gain": 0.3 },
                { "id": "hiss", "node": { "type": "noise", "color": "white" }, "pan": 0.9, "gain": 0.6,
                  "automation": [ { "target": "gain", "points": [
                      { "t": 0.0, "v": 0.1 }, { "t": 0.4, "v": 0.8 } ] } ] },
                { "id": "lead", "node": { "type": "square", "freq": 440, "duty": 0.25 }, "gain": 0.4, "at": 0.1 }
            ], "master": [ { "type": "reverb", "room": 0.3, "mix": 0.2 } ] } }"#,
    )
}

/// (c) A melodic seq on the additive piano voice (the costliest seq voice).
fn seq_piano() -> SoundDoc {
    doc(r#"{ "name": "seq-piano", "duration": 0.5, "engine": 5,
            "root": { "type": "seq", "bpm": 240, "wave": "piano",
            "env": { "a": 0.002, "s": 1.0, "r": 0.2 },
            "notes": [
                { "step": 0, "len": 4, "pitch": "C4" },
                { "step": 0, "len": 4, "pitch": "E4", "gain": 0.9 },
                { "step": 4, "len": 4, "pitch": "G3", "gain": 0.7 } ] } }"#)
}

/// (d) A heavy effects chain: unison source → filter → delay → reverb →
/// compressor (delay lines, comb/allpass tails, and envelope followers).
fn fx_chain() -> SoundDoc {
    doc(r#"{ "name": "fx-chain", "duration": 0.4, "engine": 5,
            "root": { "type": "chain", "stages": [
                { "type": "super", "freq": 110, "voices": 7, "detune_cents": 25 },
                { "type": "lowpass", "cutoff": 1200, "q": 0.9 },
                { "type": "delay", "secs": 0.09, "feedback": 0.35 },
                { "type": "reverb", "room": 0.4, "mix": 0.25 },
                { "type": "compress", "threshold": -18, "ratio": 4, "makeup": 6 } ] } }"#)
}

/// (e) A streamable graph: deterministic nodes only, constant filter
/// cutoffs, no normalize/loop/stereo — so `StreamGraph` covers it natively.
fn streamable() -> SoundDoc {
    doc(r#"{ "name": "streamable", "duration": 0.5, "engine": 5,
            "root": { "type": "chain", "stages": [
                { "type": "square", "freq": 220, "duty": 0.3 },
                { "type": "lowpass", "cutoff": 1800, "q": 0.8 },
                { "type": "drive", "amount": 1.5 },
                { "type": "tremolo", "rate": 6, "depth": 0.4 },
                { "type": "delay", "secs": 0.12, "feedback": 0.3 } ] } }"#)
}

fn bench_blip(c: &mut Criterion) {
    let doc = blip();
    c.bench_function("render/blip_osc_env", |b| {
        b.iter(|| render::render(black_box(&doc)))
    });
}

fn bench_tracks_mix(c: &mut Criterion) {
    let doc = tracks_mix();
    c.bench_function("render/tracks_mix_automation_master_reverb", |b| {
        b.iter(|| render::render_product(black_box(&doc)))
    });
}

fn bench_seq_piano(c: &mut Criterion) {
    let doc = seq_piano();
    c.bench_function("render/seq_piano", |b| {
        b.iter(|| render::render(black_box(&doc)))
    });
}

fn bench_fx_chain(c: &mut Criterion) {
    let doc = fx_chain();
    c.bench_function("render/fx_chain_reverb_delay_compress", |b| {
        b.iter(|| render::render(black_box(&doc)))
    });
}

fn bench_stream_fill(c: &mut Criterion) {
    const BLOCK: usize = 512;
    let doc = streamable();
    let n = (doc.duration * doc.sample_rate as f32).ceil() as usize;
    let blocks = n.div_ceil(BLOCK);
    c.bench_function("streaming/streamgraph_fill_512", |b| {
        b.iter(|| {
            // Rebuild per iteration so every measured run starts at position 0;
            // the doc itself is built once, outside the closure.
            let mut graph = StreamGraph::try_from_doc(black_box(&doc)).expect("doc is streamable");
            let mut block = [0.0f32; BLOCK];
            for _ in 0..blocks {
                graph.fill(black_box(&mut block));
            }
            black_box(block)
        })
    });
}

fn amp() -> Adsr {
    Adsr {
        a: 0.005,
        d: 0.1,
        s: 0.8,
        r: 0.2,
        punch: 0.0,
    }
}

/// (f) A representative multi-track song: four seq tracks over four bars
/// with a named section — the shape `Song::compile` and the runtime
/// benches share.
fn bench_song() -> Song {
    let mut song = Song::new("bench-song", 128.0);
    song.add_track("bass", SeqWave::Bass, amp());
    song.add_track("keys", SeqWave::Epiano, amp());
    song.add_track("lead", SeqWave::Square, amp());
    song.add_track("arp", SeqWave::Sawtooth, amp());
    song.add_pattern(
        "riff",
        1,
        vec![note(0, 4, "C2"), note(8, 4, "G2"), note(12, 2, "A#2")],
    );
    song.add_pattern(
        "stab",
        1,
        vec![note(0, 2, "C4"), note(6, 2, "D#4"), note(10, 2, "G4")],
    );
    song.add_pattern(
        "line",
        1,
        vec![
            note(0, 4, "C5"),
            note(4, 4, "D5"),
            note(8, 4, "D#5"),
            note(12, 4, "G5"),
        ],
    );
    song.add_pattern(
        "run",
        1,
        vec![
            note(0, 1, "C4"),
            note(2, 1, "D4"),
            note(4, 1, "E4"),
            note(6, 1, "G4"),
            note(8, 1, "A4"),
            note(10, 1, "C5"),
            note(12, 1, "D5"),
            note(14, 1, "E5"),
        ],
    );
    song.arrange_repeat("bass", "riff", 0, 4);
    song.arrange_repeat("keys", "stab", 0, 4);
    song.arrange_repeat("lead", "line", 0, 4);
    song.arrange_repeat("arp", "run", 0, 4);
    song.sections.push(tono_core::song::Section {
        name: "b".into(),
        bar: 2,
        bars: 2,
    });
    song
}

/// (g) Eight tracks over four bars, two of them routed to a drum bus with a
/// reverb insert — the stem-mixing shape.
fn stems_song() -> Song {
    let mut song = Song::new("bench-stems", 120.0);
    let tracks = [
        ("kick", SeqWave::Square, "C2"),
        ("snare", SeqWave::Noise, "G2"),
        ("bass", SeqWave::Bass, "C2"),
        ("pad", SeqWave::Triangle, "C3"),
        ("keys", SeqWave::Epiano, "E3"),
        ("lead", SeqWave::Sawtooth, "G3"),
        ("hat", SeqWave::Fm, "C4"),
        ("pluck", SeqWave::Pluck, "E4"),
    ];
    for (name, wave, pitch) in tracks {
        song.add_track(name, wave, amp());
        song.add_pattern(name, 1, vec![note(0, 4, pitch), note(8, 4, pitch)]);
        song.arrange_repeat(name, name, 0, 4);
    }
    song.buses.push(Bus {
        id: "drums".into(),
        gain: 0.9,
        effects: vec![
            serde_json::from_str::<tono_core::dsl::Node>(
                r#"{ "type": "reverb", "room": 0.5, "mix": 0.3 }"#,
            )
            .expect("bus insert parses"),
        ],
    });
    song.tracks[0].bus = Some("drums".into());
    song.tracks[1].bus = Some("drums".into());
    song
}

fn bench_compile_song(c: &mut Criterion) {
    let song = bench_song();
    let opts = CompileOptions::default();
    c.bench_function("compile/song_to_program", |b| {
        b.iter(|| {
            black_box(&song)
                .compile(black_box(&opts))
                .expect("compiles")
        })
    });
}

fn bench_performance_fill(c: &mut Criterion) {
    let program = Arc::new(
        bench_song()
            .compile(&CompileOptions::default())
            .expect("compiles"),
    );
    c.bench_function("scheduling/performance_fill_512", |b| {
        b.iter_batched(
            || {
                // Setup (untimed): a fresh Performance at frame 0, so the
                // measured fill always covers real content and a command
                // execution — never the past-the-end silence path.
                let mut p = Performance::new(program.clone());
                p.schedule(Command::Play, At::Immediate).expect("play");
                (p, vec![0.0f32; 512 * 2])
            },
            |(mut p, mut block)| {
                p.fill(black_box(&mut block));
                black_box(block)
            },
            BatchSize::SmallInput,
        )
    });
}

fn bench_stems_8(c: &mut Criterion) {
    let program = stems_song()
        .compile(&CompileOptions::default())
        .expect("compiles");
    c.bench_function("mixing/tracks_stems_8", |b| {
        b.iter(|| black_box(&program).render_stems())
    });
}

criterion_group!(
    benches,
    bench_blip,
    bench_tracks_mix,
    bench_seq_piano,
    bench_fx_chain,
    bench_stream_fill,
    bench_compile_song,
    bench_performance_fill,
    bench_stems_8
);
criterion_main!(benches);
