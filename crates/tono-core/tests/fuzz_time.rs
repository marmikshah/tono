//! Property tests for the exact-time walks (issue #52, workstream 9) — the
//! ADR 0002 contract that musical time crosses to audio frames in exactly one
//! specified way, so the compiler and the runtime can never disagree:
//!
//!   1. `units::beat_at_bar` is non-decreasing in bar — meter maps and
//!      pickups can reshape bars, but a later barline never moves earlier.
//!   2. Constant tempo, no maps: `Transport::frame_at_beat` and
//!      `units::beat_to_frames` are THE SAME conversion (same 60/bpm
//!      expression, same halves-away-from-zero rounding, same degenerate
//!      clamps) — asserted bit-exact, because a drift here IS the bug class
//!      ADR 0002 exists to prevent.
//!   3. `dsl::tempo_map_seconds_at` is non-decreasing in beat, and
//!      `dsl::tempo_map_beat_at_seconds` inverts it within 1e-9 — the map is
//!      a continuous piecewise-linear walk, so the round-trip is exact up to
//!      f64 dust.
//!   4. Plain meter: `units::bar_count_at_beat` equals the compiler's legacy
//!      ceil — `note_end.div_ceil(beats_per_bar × steps_per_beat)` from
//!      song/compile.rs `length_bars`, restated in beats (`note_end` steps at
//!      `steps_per_beat` is `Beat::new(note_end, steps_per_beat)`).
//!
//! Case budget 128; everything here is pure arithmetic.

use proptest::prelude::*;
use tono_core::dsl::{TempoPoint, tempo_map_beat_at_seconds, tempo_map_seconds_at};
use tono_core::program::Program;
use tono_core::runtime::Transport;
use tono_core::units::{
    Beat, Frames, MeterPoint, SampleRate, Tempo, bar_count_at_beat, beat_at_bar, beat_to_frames,
};

fn config() -> ProptestConfig {
    ProptestConfig {
        cases: 128,
        // Keep the repo tree clean: failures print the seed instead of
        // writing a proptest-regressions file.
        failure_persistence: None,
        ..ProptestConfig::default()
    }
}

/// A well-formed meter map: the first point at bar 0 (required when present),
/// bars strictly increasing, power-of-two denominators.
fn arb_meter_map() -> BoxedStrategy<Vec<MeterPoint>> {
    proptest::option::of(proptest::collection::vec(
        (
            1..=12u32,
            proptest::sample::select(vec![2u32, 4, 8, 16]),
            1..=4u32,
        ),
        1..=4,
    ))
    .prop_map(|opt| {
        let mut bar = 0u32;
        opt.unwrap_or_default()
            .into_iter()
            .map(|(numerator, denominator, gap)| {
                let point = MeterPoint {
                    bar,
                    numerator,
                    denominator,
                };
                bar += gap;
                point
            })
            .collect()
    })
    .boxed()
}

/// A well-formed tempo map: the first point at beat 0 (required), beats
/// strictly increasing, musical tempos.
fn arb_tempo_map() -> BoxedStrategy<Vec<TempoPoint>> {
    (
        20.0f32..=300.0,
        proptest::collection::vec((1..=8i64, 20.0f32..=300.0), 0..=4),
    )
        .prop_map(|(bpm0, tail)| {
            let mut beat = 0i64;
            let mut map = vec![TempoPoint {
                at: Beat::zero(),
                bpm: bpm0,
            }];
            for (gap, bpm) in tail {
                beat += gap;
                map.push(TempoPoint {
                    at: Beat::from_int(beat),
                    bpm,
                });
            }
            map
        })
        .boxed()
}

fn program(tempo_bpm: f32, sample_rate: u32) -> Program {
    let mut song = tono_core::song::Song::new("fuzz-time", tempo_bpm.max(1.0))
        .add(tono_core::catalog::Bass::finger().named("bass"), |_| {});
    song.tracks[0].notes.push(tono_core::song::note(0, 1, "C2"));
    song.compile(&tono_core::song::CompileOptions {
        sample_rate: Some(sample_rate),
        ..Default::default()
    })
    .unwrap()
}

proptest! {
    #![proptest_config(config())]

    /// Contract 1: barlines never move earlier as the bar index grows, under
    /// arbitrary meter maps and pickups.
    #[test]
    fn beat_at_bar_is_non_decreasing(
        map in arb_meter_map(),
        default_numerator in 1..=8u32,
        pickup in proptest::option::of((0..=3i64, 1..=4u32).prop_map(|(n, d)| Beat::new(n, d))),
        bars in proptest::collection::vec(0..=64u32, 2..=12),
    ) {
        let mut bars = bars;
        bars.sort_unstable();
        let beats: Vec<Beat> = bars
            .iter()
            .map(|&bar| beat_at_bar(&map, default_numerator, pickup, bar))
            .collect();
        for (i, pair) in beats.windows(2).enumerate() {
            prop_assert!(
                pair[0] <= pair[1],
                "bar {} starts after bar {}",
                bars[i],
                bars[i + 1]
            );
        }
    }

    /// Contract 2: the Transport's constant-tempo frame math IS
    /// `beat_to_frames` — bit-exact, including degenerate tempos (floored at
    /// 1 BPM) and negative beats (clamped to frame 0).
    #[test]
    fn transport_and_units_agree_on_constant_tempo(
        bpm in 0.0f32..=300.0,
        rate in proptest::sample::select(vec![8_000u32, 44_100, 48_000]),
        beat in (-64i64..=512, 1..=16u32).prop_map(|(n, d)| Beat::new(n, d)),
    ) {
        let transport = Transport::for_program(&program(bpm, rate));
        prop_assert_eq!(
            Frames(transport.frame_at_beat(beat.to_f64())),
            beat_to_frames(beat, Tempo(bpm), SampleRate(rate)),
            "Transport::frame_at_beat and units::beat_to_frames disagree"
        );
    }

    /// Contract 3: the tempo-map seconds walk is non-decreasing in beat, and
    /// its inverse recovers the beat to 1e-9.
    #[test]
    fn tempo_map_walk_is_monotone_and_invertible(
        map in arb_tempo_map(),
        beats in proptest::collection::vec(0.0f64..=64.0, 2..=10),
    ) {
        let mut beats = beats;
        beats.sort_by(f64::total_cmp);
        let seconds: Vec<f64> = beats
            .iter()
            .map(|&b| tempo_map_seconds_at(&map, b))
            .collect();
        for pair in seconds.windows(2) {
            prop_assert!(pair[0] <= pair[1], "seconds went backwards along the beat axis");
        }
        for (&beat, &secs) in beats.iter().zip(&seconds) {
            let back = tempo_map_beat_at_seconds(&map, secs);
            prop_assert!(
                (back - beat).abs() <= 1e-9,
                "inversion failed: beat {beat} -> {secs} s -> {back}"
            );
        }
    }

    /// Contract 4: plain-meter `bar_count_at_beat` is the compiler's legacy
    /// ceil, restated in beats. (`note_end` steps at `spb` steps/beat is the
    /// beat `note_end/spb`; the legacy formula ceils it against
    /// `beats_per_bar × steps_per_beat` steps.)
    #[test]
    fn bar_count_at_beat_matches_the_legacy_ceil(
        note_end in 0..=512u32,
        spb in 1..=8u32,
        bpb in 1..=8u32,
    ) {
        prop_assert_eq!(
            bar_count_at_beat(&[], bpb, None, Beat::new(note_end as i64, spb)),
            note_end.div_ceil(bpb * spb),
            "bar_count_at_beat diverged from length_bars' ceil semantics"
        );
    }
}
