//! Property-based fuzzing of the Performance command stream (issue #52,
//! workstream 9) — the contract pinned here:
//!
//!   1. `fill` never panics and every output sample is finite, under
//!      arbitrary legal command scripts (Play / Pause / Stop / SeekBar /
//!      SeekBeat / SetLoopBars / ClearLoop / SetGain at Immediate / Frame /
//!      Beat / Bar times, scheduled in batches interleaved with rendering).
//!      Swap and Stinger stay out of the strategy — they need payloads.
//!   2. Determinism under arbitrary scheduling: the same script rendered
//!      twice through fresh Performances is byte-identical. Scripts without
//!      gain rides are additionally byte-identical across DIFFERENT host
//!      block sizes — command execution is frame-exact and the song source
//!      is position-driven, so host blocking can't leak into the output
//!      (gain ramps interpolate across each slice, so a scripted `SetGain`
//!      is only pinned under identical blocking).
//!   3. Metrics stay consistent: executed + current queue depth == accepted
//!      commands and dropped == rejected at every point, and the counters
//!      never go backwards.
//!
//! Programs are tiny (1–2 tracks, 2–4 bars at 8 kHz) so a whole case —
//! program compile plus two full runs — stays cheap; case budget 64 like the
//! validation fuzz.

use std::sync::Arc;

use proptest::prelude::*;
use tono_core::dsl::{Adsr, SeqWave};
use tono_core::program::Program;
use tono_core::runtime::performance::{At, Command, Performance};
use tono_core::runtime::{AudioSource, Transport};
use tono_core::song::{CompileOptions, Song, note};

fn config() -> ProptestConfig {
    ProptestConfig {
        cases: 64,
        // Keep the repo tree clean: failures print the seed instead of
        // writing a proptest-regressions file.
        failure_persistence: None,
        ..ProptestConfig::default()
    }
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

const PITCHES: &[&str] = &["C2", "E2", "G2", "A2", "C3", "midi:36", "midi:38"];

/// A small program: 1–2 tracks with 1-bar patterns repeated over 1–2 bars,
/// compiled at 8 kHz so the buffer-backed source is a cheap render.
fn arb_program() -> BoxedStrategy<Arc<Program>> {
    (
        proptest::sample::select(vec![100.0f32, 120.0, 140.0]),
        1..=2usize,
        1..=2u32,
        proptest::collection::vec(
            (0..16u32, 1..=8u32, proptest::sample::select(PITCHES)),
            2..=5,
        ),
        proptest::collection::vec(
            (0..16u32, 1..=8u32, proptest::sample::select(PITCHES)),
            1..=4,
        ),
    )
        .prop_map(|(bpm, n_tracks, bars, riff, stab)| {
            let mut song = Song::new("fuzz-cmd", bpm);
            song.add_track("t0", SeqWave::Square, amp());
            if n_tracks == 2 {
                song.add_track("t1", SeqWave::Bass, amp());
            }
            song.add_pattern(
                "riff",
                1,
                riff.iter()
                    .map(|(s, l, p)| note(*s, *l, p))
                    .collect::<Vec<_>>(),
            );
            song.arrange_repeat("t0", "riff", 0, bars);
            if n_tracks == 2 {
                song.add_pattern(
                    "stab",
                    1,
                    stab.iter()
                        .map(|(s, l, p)| note(*s, *l, p))
                        .collect::<Vec<_>>(),
                );
                song.arrange_repeat("t1", "stab", 0, bars);
            }
            let options = CompileOptions {
                sample_rate: Some(8_000),
                ..CompileOptions::default()
            };
            Arc::new(song.compile(&options).expect("the fuzz song compiles"))
        })
        .boxed()
}

/// Any schedulable time within sane bounds: immediate, an absolute frame
/// inside/past the render span, an absolute beat, or an absolute bar.
fn arb_at() -> BoxedStrategy<At> {
    prop_oneof![
        2 => Just(At::Immediate),
        3 => (0..=48_000u64).prop_map(At::Frame),
        2 => (0.0f64..=24.0).prop_map(At::Beat),
        2 => (0..=6u32).prop_map(At::Bar),
    ]
    .boxed()
}

/// Any payload-free command, per the fuzz scope.
fn arb_command() -> BoxedStrategy<Command> {
    prop_oneof![
        2 => Just(Command::Play),
        1 => Just(Command::Pause),
        1 => Just(Command::Stop),
        1 => (0..=6u32).prop_map(Command::SeekBar),
        1 => (0.0f64..=24.0).prop_map(Command::SeekBeat),
        1 => (0..=4u32, 0..=6u32).prop_map(|(a, b)| Command::SetLoopBars(a, b)),
        1 => Just(Command::ClearLoop),
        2 => (0.0f32..=2.0).prop_map(Command::SetGain),
    ]
    .boxed()
}

/// A script without `SetGain` — the class whose output is independent of the
/// host's block size (no ramps to resample across slice boundaries).
fn arb_command_no_gain() -> BoxedStrategy<Command> {
    prop_oneof![
        2 => Just(Command::Play),
        1 => Just(Command::Pause),
        1 => Just(Command::Stop),
        1 => (0..=6u32).prop_map(Command::SeekBar),
        1 => (0.0f64..=24.0).prop_map(Command::SeekBeat),
        1 => (0..=4u32, 0..=6u32).prop_map(|(a, b)| Command::SetLoopBars(a, b)),
        1 => Just(Command::ClearLoop),
    ]
    .boxed()
}

/// One metrics consistency snapshot after a fill.
#[derive(Debug)]
struct Snapshot {
    executed: u64,
    depth: u64,
    dropped: u64,
    accepted: u64,
    rejected: u64,
}

/// A full scripted run: schedule the script in batches interleaved with
/// fills (so `At::Immediate` lands at many different clocks), then drain to
/// `total_frames`. With `schedule_up_front` every command is scheduled before
/// the first fill — required when comparing across DIFFERENT block sizes,
/// because `At::Immediate` resolves against the clock at schedule time and an
/// interleaved cadence would make that clock depend on the blocking.
struct Run {
    samples: Vec<f32>,
    snapshots: Vec<Snapshot>,
}

fn run_script(
    program: &Arc<Program>,
    script: &[(Command, At)],
    total_frames: usize,
    block: usize,
    schedule_up_front: bool,
) -> Run {
    let mut p = Performance::new(program.clone());
    let mut samples = Vec::with_capacity(total_frames * 2);
    let mut snapshots = Vec::new();
    let (mut accepted, mut rejected) = (0u64, 0u64);
    let mut next = 0usize;
    // A batch of up to 3 commands per block — scheduling cadence is part of
    // the run, so identical blockings reproduce identical schedules.
    let batch = if schedule_up_front { script.len() } else { 3 };
    while samples.len() < total_frames * 2 {
        for _ in 0..batch {
            if next >= script.len() {
                break;
            }
            let (command, at) = &script[next];
            next += 1;
            match p.schedule(command.clone(), at.clone()) {
                Ok(_) => accepted += 1,
                Err(_) => rejected += 1,
            }
        }
        let take = block.min(total_frames - samples.len() / 2);
        let mut buf = vec![0.0f32; take * 2];
        let filled = p.fill(&mut buf);
        assert_eq!(filled, take, "fill must produce the requested frame count");
        samples.extend_from_slice(&buf);
        let m = p.metrics();
        snapshots.push(Snapshot {
            executed: m.commands_executed,
            depth: p.queue_depth() as u64,
            dropped: m.commands_dropped,
            accepted,
            rejected,
        });
    }
    Run { samples, snapshots }
}

/// The metrics contract over one run's snapshots: consistency at every point
/// and monotone counters.
fn assert_metrics_contract(run: &Run) -> Result<(), TestCaseError> {
    let mut prev: Option<&Snapshot> = None;
    for snap in &run.snapshots {
        prop_assert_eq!(
            snap.executed + snap.depth,
            snap.accepted,
            "executed + queue depth must equal the accepted commands"
        );
        prop_assert_eq!(
            snap.dropped,
            snap.rejected,
            "dropped must equal the rejected commands"
        );
        if let Some(prev) = prev {
            prop_assert!(
                snap.executed >= prev.executed && snap.dropped >= prev.dropped,
                "counters went backwards: {prev:?} -> {snap:?}"
            );
        }
        prev = Some(snap);
    }
    Ok(())
}

fn bits(samples: &[f32]) -> Vec<u32> {
    samples.iter().map(|x| x.to_bits()).collect()
}

fn assert_finite(samples: &[f32]) -> Result<(), TestCaseError> {
    prop_assert!(
        samples.iter().all(|s| s.is_finite()),
        "non-finite sample in the performance output"
    );
    Ok(())
}

/// Host block sizes: odd/prime-ish values to cross command frames awkwardly,
/// plus the usual powers of two.
const BLOCKS: &[usize] = &[1, 37, 256, 512, 1000, 2048];

proptest! {
    #![proptest_config(config())]

    /// Contract 1 + 2 + 3: arbitrary scripts never panic and stay finite;
    /// the same script run twice (fresh Performances, identical blocking) is
    /// byte-identical; metrics stay consistent and monotone throughout.
    #[test]
    fn scripted_performance_is_deterministic(
        program in arb_program(),
        script in proptest::collection::vec((arb_command(), arb_at()), 1..=24),
        total_frames in 6_000usize..=12_000,
        block in proptest::sample::select(BLOCKS.to_vec()),
    ) {
        let a = run_script(&program, &script, total_frames, block, false);
        let b = run_script(&program, &script, total_frames, block, false);
        assert_finite(&a.samples)?;
        assert_eq!(
            bits(&a.samples),
            bits(&b.samples),
            "the same script rendered twice diverged"
        );
        assert_metrics_contract(&a)?;
        assert_metrics_contract(&b)?;
    }

    /// Contract 2b: without gain rides, the same script is byte-identical
    /// across DIFFERENT host block sizes — all commands are scheduled up
    /// front (so `At` resolution can't depend on the blocking) and every
    /// command lands on its exact frame regardless of how the host slices
    /// the render.
    #[test]
    fn gainless_scripts_are_block_size_independent(
        program in arb_program(),
        script in proptest::collection::vec((arb_command_no_gain(), arb_at()), 1..=16),
        total_frames in 6_000usize..=10_000,
        block_a in proptest::sample::select(BLOCKS.to_vec()),
        block_b in proptest::sample::select(BLOCKS.to_vec()),
    ) {
        // Sanity: the program's own frame math is reachable mid-script (a
        // seek target the script can actually land on).
        let transport = Transport::for_program(&program);
        prop_assert!(transport.frame_at_beat(0.0) == 0);
        let a = run_script(&program, &script, total_frames, block_a, true);
        let b = run_script(&program, &script, total_frames, block_b, true);
        assert_finite(&a.samples)?;
        assert_finite(&b.samples)?;
        assert_eq!(
            bits(&a.samples),
            bits(&b.samples),
            "block sizes {block_a}/{block_b} diverged on a gainless script"
        );
        assert_metrics_contract(&a)?;
        assert_metrics_contract(&b)?;
    }
}
