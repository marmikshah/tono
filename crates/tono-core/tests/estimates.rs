//! Resource estimates bound reality (issue #52, workstream 9): for
//! representative programs, `Program::estimates` must agree with what the
//! renderer actually produces.
//!
//! The pinned frames relationship (the exact rounding contract):
//!
//! - `estimates.frames` is
//!   `round(duration × sample_rate)` — `duration_frames` in song/compile.rs.
//! - The renderer's actual frame count (`render_stereo` channel length) is
//!   `ceil(duration × sample_rate)` with a one-frame floor — render/mod.rs
//!   (`render_plain`) and render/tracks.rs (`render_tracks_impl`).
//!
//! For x = duration × sr > 0, `ceil(x) − round(x)` is exactly 1 when
//! `fract(x) ∈ (0, 0.5)` and 0 otherwise (Rust's `round` rounds half away
//! from zero, landing on `ceil` whenever the fraction is ≥ 0.5). So the
//! estimate equals the true render length or undershoots it by exactly one
//! frame — never more. `memory_bytes` is exactly `frames × 8` (the stereo
//! f32 output buffers), so against the TRUE render length it can undershoot
//! by that same one frame (8 bytes) — pinned below, not smoothed over.

use tono_core::dsl::{Adsr, SeqWave};
use tono_core::program::Program;
use tono_core::song::{CompileOptions, Song, note};

fn amp() -> Adsr {
    Adsr {
        a: 0.005,
        d: 0.1,
        s: 0.8,
        r: 0.2,
        punch: 0.0,
    }
}

/// One near-empty program: a single note on a single track.
fn empty_ish() -> Song {
    let mut song = Song::new("empty-ish", 120.0);
    song.add_track("tone", SeqWave::Sine, amp());
    song.tracks[0].notes.push(note(0, 4, "C4"));
    song
}

/// A dense 16-track song: every track plays the same 8-note pattern (chords
/// of up to 3 simultaneous notes) for 8 bars — 1024 events, per-track peak
/// 3, and the per-track peaks coincide.
fn dense_16() -> Song {
    let mut song = Song::new("dense-16", 120.0);
    let pattern = vec![
        note(0, 4, "C3"),
        note(0, 4, "E3"),
        note(4, 4, "G3"),
        note(8, 2, "C4"),
        note(8, 2, "E4"),
        note(8, 2, "G4"),
        note(12, 2, "D4"),
        note(14, 2, "B3"),
    ];
    for t in 0..16 {
        song.add_track(format!("t{t}"), SeqWave::Sawtooth, amp());
        song.add_pattern(format!("p{t}"), 1, pattern.clone());
        song.arrange_repeat(&format!("t{t}"), &format!("p{t}"), 0, 8);
    }
    song
}

/// A long ambient piece: two pads in call-and-response (never overlapping),
/// 90 BPM, 8 bars — per-track peaks of 1 each, so the summed estimate (2)
/// strictly exceeds the true simultaneous maximum (1).
fn long_ambient() -> Song {
    let mut song = Song::new("long-ambient", 90.0);
    song.add_track("pad1", SeqWave::Triangle, amp());
    song.add_track("pad2", SeqWave::Sine, amp());
    // 32 steps = 8 beats = 2 bars at 4 steps/beat.
    song.tracks[0].notes.push(note(0, 32, "C3"));
    song.tracks[0].notes.push(note(64, 32, "G3"));
    song.tracks[1].notes.push(note(32, 32, "E3"));
    song.tracks[1].notes.push(note(96, 32, "B3"));
    song
}

/// A program whose `duration × sr` fraction lands in (0, 0.5): the one case
/// where the ceil render length overshoots the rounded estimate by exactly
/// one frame. 77 BPM, one 8-step note: duration = 8 × 60/(77×4) + 2 ≈
/// 3.5584416 s — × 48 000 ≈ 170 805.2 frames (fraction ≈ 0.2).
fn fractional_frames() -> (Song, CompileOptions) {
    let mut song = Song::new("fractional", 77.0);
    song.add_track("tone", SeqWave::Sine, amp());
    song.tracks[0].notes.push(note(0, 8, "A3"));
    (
        song,
        CompileOptions {
            sample_rate: Some(48_000),
            ..CompileOptions::default()
        },
    )
}

/// The compiled note spans of every track: direct notes plus arrangement
/// placements, as half-open [start, end) steps — recomputed from the song,
/// independently of the compiler's own span code.
fn note_spans(song: &Song) -> Vec<Vec<(u32, u32)>> {
    let steps_per_bar = song.beats_per_bar.max(1) * song.steps_per_beat.max(1);
    song.tracks
        .iter()
        .map(|t| {
            let end = |n: &tono_core::dsl::SeqNote| (n.step, n.step + n.len.max(1));
            let mut spans: Vec<(u32, u32)> = t.notes.iter().map(&end).collect();
            for pl in song.arrangement.iter().filter(|p| p.track == t.name) {
                let pat = song
                    .patterns
                    .iter()
                    .find(|p| p.name == pl.pattern)
                    .expect("arranged pattern exists");
                let offset = pl.bar * steps_per_bar;
                spans.extend(pat.notes.iter().map(|n| {
                    let (s, e) = end(n);
                    (s + offset, e + offset)
                }));
            }
            spans
        })
        .collect()
}

/// The largest number of half-open spans covering any step — an independent
/// re-implementation of the overlap sweep (ends sort before starts at a
/// shared step, so back-to-back notes never count as simultaneous).
fn peak_overlap(spans: &[(u32, u32)]) -> u32 {
    let mut points: Vec<(u32, i64)> = spans
        .iter()
        .flat_map(|(s, e)| [(*s, 1), (*e, -1)])
        .collect();
    points.sort();
    let (mut current, mut peak) = (0, 0);
    for (_, delta) in points {
        current += delta;
        peak = peak.max(current);
    }
    peak.max(0) as u32
}

fn check(song: &Song, opts: &CompileOptions) -> Program {
    let program = song.compile(opts).expect("compiles");
    let est = &program.estimates;
    let duration = program.doc.duration;
    let sr = program.doc.sample_rate;

    // The pinned rounding contract, both sides exact.
    let expected_estimate = (duration * sr as f32).round().max(0.0) as u64;
    assert_eq!(est.frames, expected_estimate, "estimates.frames rounds");
    let (left, right) = program.render_stereo();
    assert_eq!(left.len(), right.len(), "stereo channels agree");
    let actual = left.len() as u64;
    assert_eq!(
        actual as usize,
        ((duration.clamp(0.0, 600.0) * sr as f32).ceil() as usize).max(1),
        "the renderer ceils, with a one-frame floor"
    );
    let frac = (duration * sr as f32).fract();
    let gap = if frac > 0.0 && frac < 0.5 { 1 } else { 0 };
    assert_eq!(
        actual,
        est.frames + gap,
        "ceil(x) − round(x) is 1 exactly when fract(x) ∈ (0, 0.5)"
    );

    // The memory estimate is exactly the stereo output buffers of the
    // ESTIMATED frame count — and never more than one frame (8 bytes) short
    // of the true render's buffers.
    assert_eq!(est.memory_bytes, est.frames * 8, "the exact definition");
    assert!(
        est.memory_bytes >= 2 * est.frames * 4,
        "bounds the estimate"
    );
    assert!(
        est.memory_bytes + 8 >= 2 * actual * 4,
        "within one frame of the true stereo buffers (the ceil/round gap)"
    );

    // Events and voices, recomputed from the song's own notes.
    let spans = note_spans(song);
    let events: u64 = spans.iter().map(|s| s.len() as u64).sum();
    assert_eq!(est.events, events, "events == actual note count");
    assert_eq!(
        est.events,
        program
            .meta
            .tracks
            .iter()
            .map(|t| t.notes as u64)
            .sum::<u64>(),
        "meta track note counts agree"
    );
    let per_track: u32 = spans.iter().map(|s| peak_overlap(s)).sum();
    assert_eq!(
        est.peak_voices, per_track,
        "the estimate sums the per-track peaks"
    );
    let all: Vec<(u32, u32)> = spans.into_iter().flatten().collect();
    let true_peak = peak_overlap(&all);
    assert!(
        est.peak_voices >= true_peak,
        "bounds the true max simultaneous notes ({true_peak})"
    );
    program
}

#[test]
fn estimates_bound_an_empty_ish_program() {
    let program = check(&empty_ish(), &CompileOptions::default());
    assert_eq!(program.estimates.events, 1);
    assert_eq!(program.estimates.peak_voices, 1);
}

#[test]
fn estimates_bound_a_dense_16_track_song() {
    let program = check(&dense_16(), &CompileOptions::default());
    assert_eq!(program.estimates.events, 16 * 8 * 8, "1024 arranged notes");
    assert_eq!(
        program.estimates.peak_voices,
        16 * 3,
        "each track peaks at 3 simultaneous notes and the peaks coincide"
    );
}

#[test]
fn estimates_bound_a_long_ambient() {
    let program = check(&long_ambient(), &CompileOptions::default());
    assert_eq!(program.estimates.events, 4);
    assert_eq!(
        program.estimates.peak_voices, 2,
        "the per-track peaks summed — strictly above the true overlap of 1"
    );
}

#[test]
fn frames_estimate_undershoots_by_exactly_one_frame_when_the_fraction_is_small() {
    let (song, opts) = fractional_frames();
    let program = check(&song, &opts);
    let x = program.doc.duration * program.doc.sample_rate as f32;
    assert!(
        x.fract() > 0.0 && x.fract() < 0.5,
        "this case exercises the +1 branch: fract({x}) = {}",
        x.fract()
    );
    let (left, _) = program.render_stereo();
    assert_eq!(
        left.len() as u64,
        program.estimates.frames + 1,
        "the ceil render length overshoots the rounded estimate by one frame"
    );
}
