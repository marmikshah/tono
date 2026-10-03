//! Property tests for the pattern transforms — the
//! musical-algebra contract of `song::pattern`, pinned on random small
//! patterns (2–16 notes at unique steps, named/"midi:N" pitches, 1–4 bars):
//!
//!   * `reverse(reverse(p)) == p` — mirroring is an involution.
//!   * `transpose(transpose(p, a), b) == transpose(p, a + b)` — shifts compose
//!     (within the MIDI range; names come back in canonical sharp spelling).
//!   * `rotate(p, total) == p` and `rotate(rotate(p, h1), h2) ==
//!     rotate(p, h1 + h2)` — a true modular rotate.
//!   * `stretch(p, 1, 1) == p` and `stretch(stretch(p, 2, 1), 1, 2) == p` —
//!     exactness means exact round-trips where the grid allows.
//!   * `quantize(quantize(p, g), g) == quantize(p, g)` — snapping is
//!     idempotent.
//!   * `probability(p, 1.0, seed) == p`, `probability(p, 0.0, seed)` is empty,
//!     and a pattern's drops don't depend on note order — the draw follows the
//!     note (step, len, pitch), never the vec position.
//!   * `humanize(p, 0.0, 0.0, seed) == p`, `vel(p, 1.0) == p`,
//!     `gate(p, 1.0) == p` — the neutral elements are exact identities.
//!   * repeat/concat/layer/slice invariants: counts and bar lengths add,
//!     `slice(p, 0, total)` keeps every note, `layer(p, p)` doubles.
//!
//! Names carry op suffixes by design (`riff_rev`, `riff_t+5`), so equality
//! here is on the MUSICAL content — (step, len, pitch, gain) per note plus
//! the bar count. Generated patterns have unique steps, which makes the
//! sorted-by-step output of the grid ops fully deterministic. Case budget
//! 128; every op is pure and fast.

use proptest::prelude::*;
use tono_core::dsl::Value;
use tono_core::song::{
    Pattern, concat, gate, humanize, layer, probability, quantize, repeat, reverse, rotate, slice,
    stretch, transpose, vel,
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

/// Sharp-spelled names within MIDI 36..=96, so any two ±12 transposes stay in
/// range (transpose never errors in these properties).
const PITCHES: &[&str] = &[
    "C2", "E2", "G2", "C3", "D#3", "F#3", "A3", "C4", "E4", "G4", "B4", "D5", "midi:36", "midi:38",
    "midi:60", "midi:69",
];

/// A pattern plus the grid it was generated for (`steps_per_bar` — the ops
/// take it explicitly). Notes are sorted with UNIQUE steps, so the ops'
/// sorted output has one canonical order.
fn arb_pattern() -> BoxedStrategy<(Pattern, u32)> {
    (1..=4u32, proptest::sample::select(vec![4u32, 8, 16]))
        .prop_flat_map(|(bars, spb)| {
            let total = (bars * spb) as usize;
            (Just(bars), Just(spb), 2..=total.min(16))
        })
        .prop_flat_map(|(bars, spb, count)| {
            let total = bars * spb;
            (
                // Unique, in-order steps sampled as a subsequence — a
                // `hash_set` of near-full ranges rejects itself to death.
                proptest::sample::subsequence((0..total).collect::<Vec<u32>>(), count),
                proptest::collection::vec(1..=8u32, count),
                proptest::collection::vec(proptest::sample::select(PITCHES), count),
                proptest::collection::vec(0.0f32..=1.0, count),
                Just(bars),
                Just(spb),
            )
        })
        .prop_map(|(steps, lens, pitches, gains, bars, spb)| {
            let total = bars * spb;
            let notes = steps
                .into_iter()
                .zip(lens)
                .zip(pitches)
                .zip(gains)
                .map(|(((step, len), pitch), gain)| {
                    // No note overruns the pattern end: inside the pattern a
                    // reverse is a true mirror (the involution's domain). The
                    // documented saturating case is pinned separately below.
                    let len = len.min(total - step);
                    tono_core::song::note_vel(step, len, pitch, gain)
                })
                .collect();
            (
                Pattern {
                    name: "p".into(),
                    bars,
                    notes,
                },
                spb,
            )
        })
        .boxed()
}

/// The documented exception to the involution the strategy above stays clear
/// of: a note poking past the pattern end saturates onto step 0 rather than
/// wrapping (pattern.rs), so its mirror does NOT round-trip — by design.
#[test]
fn reverse_saturates_notes_past_the_pattern_end() {
    let p = Pattern {
        name: "p".into(),
        bars: 1,
        notes: vec![tono_core::song::note(12, 8, "C2")], // occupies [12, 20) of 16
    };
    let once = reverse(&p, 16);
    assert_eq!(once.notes[0].step, 0, "the overrun saturates to step 0");
    let twice = reverse(&once, 16);
    assert_eq!(
        twice.notes[0].step, 8,
        "the saturated mirror lands at total - len, not the original step"
    );
}

/// A note's musical content as a comparable key (patterns don't implement
/// PartialEq — names intentionally differ after transforms).
fn key(p: &Pattern) -> Vec<(u32, u32, String, u32)> {
    p.notes
        .iter()
        .map(|n| {
            let pitch = match &n.pitch {
                Value::Note(s) => s.clone(),
                Value::Const(hz) => format!("{hz}"),
                Value::Modulated(_) => "mod".into(),
            };
            (n.step, n.len, pitch, n.gain.to_bits())
        })
        .collect()
}

/// Content equality: same notes, same bar count (names are labels, not data).
fn assert_same(p: &Pattern, q: &Pattern) -> Result<(), TestCaseError> {
    prop_assert_eq!(key(p), key(q), "notes differ");
    prop_assert_eq!(p.bars, q.bars, "bar counts differ");
    Ok(())
}

proptest! {
    #![proptest_config(config())]

    /// Mirroring is an involution: reverse(reverse(p)) == p.
    #[test]
    fn reverse_is_an_involution((p, spb) in arb_pattern()) {
        let twice = reverse(&reverse(&p, spb), spb);
        assert_same(&p, &twice)?;
    }

    /// Transposes compose: transpose(transpose(p, a), b) == transpose(p, a+b).
    /// The pitch pool keeps a+b ∈ ±24 inside the MIDI range, so neither side
    /// errors.
    #[test]
    fn transpose_composes(
        (p, _spb) in arb_pattern(),
        a in -12i16..=12,
        b in -12i16..=12,
    ) {
        let stepwise = transpose(&transpose(&p, a).unwrap(), b).unwrap();
        let summed = transpose(&p, a + b).unwrap();
        assert_same(&stepwise, &summed)?;
    }

    /// A full turn is the identity; rotating by halves equals rotating by the
    /// sum (modular arithmetic on note starts).
    #[test]
    fn rotate_is_modular(
        (p, spb) in arb_pattern(),
        h1 in -96i64..=96,
        h2 in -96i64..=96,
    ) {
        let total = i64::from(p.bars * spb);
        assert_same(&p, &rotate(&p, total, spb))?;
        assert_same(
            &rotate(&rotate(&p, h1, spb), h2, spb),
            &rotate(&p, h1 + h2, spb),
        )?;
    }

    /// stretch 1/1 is the identity; doubling then halving returns the exact
    /// original (the doubling makes every value even, so the halving is
    /// always on-grid).
    #[test]
    fn stretch_round_trips_exactly((p, spb) in arb_pattern()) {
        assert_same(&p, &stretch(&p, 1, 1, spb).unwrap())?;
        let doubled = stretch(&p, 2, 1, spb).unwrap();
        let halved = stretch(&doubled, 1, 2, spb).unwrap();
        assert_same(&p, &halved)?;
    }

    /// Snapped starts are already on the grid: quantize is idempotent.
    #[test]
    fn quantize_is_idempotent((p, _spb) in arb_pattern(), g in 1..=16u32) {
        let once = quantize(&p, g);
        let twice = quantize(&once, g);
        assert_same(&once, &twice)?;
    }

    /// keep 1.0 keeps everything, 0.0 drops everything, and the drops follow
    /// the NOTES: reordering the input (reversed here) changes nothing about
    /// which notes survive — only their order in the output vec.
    #[test]
    fn probability_bounds_and_note_identity(
        (p, _spb) in arb_pattern(),
        keep in 0.0f32..=1.0,
        seed in any::<u64>(),
    ) {
        assert_same(&p, &probability(&p, 1.0, seed))?;
        prop_assert!(probability(&p, 0.0, seed).notes.is_empty());
        // Same seed twice: identical result (determinism).
        assert_same(
            &probability(&p, keep, seed),
            &probability(&p, keep, seed),
        )?;
        // Reversed input: same survivors, as a set.
        let mut reversed = p.clone();
        reversed.notes.reverse();
        let mut a = key(&probability(&p, keep, seed));
        let mut b = key(&probability(&reversed, keep, seed));
        a.sort();
        b.sort();
        prop_assert_eq!(a, b, "the drops must follow the notes, not the vec order");
    }

    /// Zero amounts are exact identities for the seeded jitter, and the
    /// jitter itself is deterministic in the seed.
    #[test]
    fn humanize_neutral_and_deterministic(
        (p, _spb) in arb_pattern(),
        timing in 0.0f32..=1.0,
        velocity in 0.0f32..=1.0,
        seed in any::<u64>(),
    ) {
        assert_same(&p, &humanize(&p, 0.0, 0.0, seed))?;
        assert_same(
            &humanize(&p, timing, velocity, seed),
            &humanize(&p, timing, velocity, seed),
        )?;
    }

    /// Scale-by-one is an exact identity for gains and lengths.
    #[test]
    fn vel_and_gate_have_exact_neutral_elements((p, _spb) in arb_pattern()) {
        assert_same(&p, &vel(&p, 1.0))?;
        assert_same(&p, &gate(&p, 1.0))?;
    }

    /// repeat multiplies notes and bars; concat adds them; layer(p, p)
    /// doubles the notes at the longer bar count.
    #[test]
    fn repeat_concat_layer_arithmetic(
        (a, spb) in arb_pattern(),
        (b, _) in arb_pattern(),
        times in 0..=4u32,
    ) {
        let r = repeat(&a, spb, times);
        prop_assert_eq!(r.notes.len(), a.notes.len() * times as usize);
        prop_assert_eq!(r.bars, a.bars * times);

        let c = concat(&a, &b, spb);
        prop_assert_eq!(c.notes.len(), a.notes.len() + b.notes.len());
        prop_assert_eq!(c.bars, a.bars + b.bars);
        // concat keeps a's notes untouched, in order, as a prefix.
        prop_assert_eq!(&key(&c)[..a.notes.len()], &key(&a)[..]);

        let l = layer(&a, &a);
        prop_assert_eq!(l.notes.len(), 2 * a.notes.len());
        prop_assert_eq!(l.bars, a.bars);
    }

    /// A slice over the whole pattern keeps every note (re-based at 0, which
    /// is a no-op here) and reports the pattern's own bar count.
    #[test]
    fn slice_over_the_whole_pattern_is_the_identity((p, spb) in arb_pattern()) {
        let total = p.bars * spb;
        let s = slice(&p, 0, total, spb);
        assert_same(&p, &s)?;
        // And no slice can ever produce MORE notes than its input.
        let windowed = slice(&p, total / 2, total / 2, spb);
        prop_assert!(windowed.notes.len() <= p.notes.len());
    }
}
