//! Property-based fuzzing of the Program bundle loader (issue #52, workstream
//! 9) — the contract pinned here:
//!
//!   1. `Program::from_json` NEVER panics — on arbitrary bytes, arbitrary
//!      Unicode strings, or structured near-valid mutations of a real bundle
//!      (field values scrambled, keys duplicated, the text truncated). The
//!      outcome is always `Ok` or a typed `ProgramError`
//!      (`Json` / `TooNew` / `HashMismatch`).
//!   2. When a mutated bundle still loads, it round-trips: `to_json()` is a
//!      fixpoint and re-loads to the same content hash. A hand-edited bundle
//!      whose edits dodge the doc (the hash covers only the resolved
//!      document, not the meta) is a legitimate bundle and must behave like
//!      one.
//!
//! The base bundle is compiled once through the public Song API; mutations
//! are driven by a proptest-drawn seed through the deterministic `dsp::Rng`,
//! so a failure reproduces from the printed seed alone. Case budget 128 —
//! each case is a few KB of JSON work, so the file stays fast in debug.

use proptest::prelude::*;
use serde_json::Value as J;
use tono_core::dsl::{Adsr, SeqWave};
use tono_core::dsp::Rng;
use tono_core::program::Program;
use tono_core::song::{CompileOptions, Marker, Section, Song, note};
use tono_core::units::Beat;

fn config() -> ProptestConfig {
    ProptestConfig {
        cases: 128,
        // Keep the repo tree clean: failures print the seed instead of
        // writing a proptest-regressions file.
        failure_persistence: None,
        ..ProptestConfig::default()
    }
}

/// A real bundle, built once through the public Song API: two tracks, two
/// patterns, a section and a marker — meaty JSON for the mutations to chew on.
fn base_bundle() -> &'static str {
    static BASE: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    BASE.get_or_init(|| {
        let amp = Adsr {
            a: 0.005,
            d: 0.1,
            s: 0.8,
            r: 0.2,
            punch: 0.0,
        };
        let mut song = Song::new("fuzz-bundle", 120.0);
        song.add_track("bass", SeqWave::Bass, amp);
        song.add_track("keys", SeqWave::Epiano, amp);
        song.add_pattern("riff", 1, vec![note(0, 4, "C2"), note(8, 4, "G2")]);
        song.add_pattern("stab", 1, vec![note(0, 2, "C4"), note(6, 2, "D#4")]);
        song.arrange_repeat("bass", "riff", 0, 2);
        song.arrange_repeat("keys", "stab", 0, 2);
        song.sections.push(Section {
            name: "second".into(),
            bar: 1,
            bars: 1,
        });
        song.markers.push(Marker {
            name: "hit".into(),
            at: Beat::from_int(2),
        });
        song.compile(&CompileOptions::default())
            .expect("the base song compiles")
            .to_json()
    })
}

/// One random JSON scalar — the leaf pool for the scramble pass. Numbers
/// include the bundle's own domains (versions, hashes) and their edges.
fn random_scalar(rng: &mut Rng) -> J {
    match rng.next_u64() % 16 {
        0 => J::Null,
        1 => J::Bool(true),
        2 => J::Bool(false),
        3 => J::from(0u64),
        4 => J::from(1u64),
        5 => J::from(-1i64),
        6 => J::from(u32::MAX),
        7 => J::from(u64::MAX),
        8 => J::from(i64::MIN),
        9 => J::from(0.5f64),
        10 => J::from(1e308_f64),
        11 => J::from(-1e308_f64),
        12 => J::from(""),
        13 => J::from("midi:36"),
        14 => J::Array(vec![]),
        _ => J::Object(serde_json::Map::new()),
    }
}

/// A bounded recursive value scramble: replace up to `budget` leaves (and
/// occasionally whole subtrees) with random scalars. Deterministic in `seed`.
fn scramble(value: &mut J, rng: &mut Rng, budget: &mut u32) {
    let take_leaf = |rng: &mut Rng| rng.next_u64().is_multiple_of(3);
    match value {
        J::Object(map) => {
            for val in map.values_mut() {
                if *budget == 0 {
                    return;
                }
                if take_leaf(rng) {
                    *val = random_scalar(rng);
                    *budget -= 1;
                } else {
                    scramble(val, rng, budget);
                }
            }
        }
        J::Array(items) => {
            for val in items.iter_mut() {
                if *budget == 0 {
                    return;
                }
                if take_leaf(rng) {
                    *val = random_scalar(rng);
                    *budget -= 1;
                } else {
                    scramble(val, rng, budget);
                }
            }
        }
        _ => {
            *value = random_scalar(rng);
            *budget -= 1;
        }
    }
}

/// Top-level keys worth duplicating — serde rejects a duplicate struct field,
/// so this exercises the `Json` error path with well-formed JSON text.
const DUP_KEYS: &[&str] = &["program_version", "hash", "target", "doc", "meta"];

/// Structured near-valid mutations of the real bundle's JSON text: a bounded
/// value scramble, then an optional duplicated top-level key, then optional
/// truncation at a char boundary.
fn mutated_bundle() -> BoxedStrategy<String> {
    (
        any::<u64>(),
        0..=6u32,
        proptest::option::of((proptest::sample::select(DUP_KEYS), any::<u64>())),
        proptest::option::of(0.0f64..1.0),
    )
        .prop_map(|(seed, budget, dup, trunc)| {
            let mut text = base_bundle().to_string();
            if budget > 0 {
                let mut value: J = serde_json::from_str(&text).expect("the base bundle parses");
                let mut rng = Rng::new(seed);
                let mut budget = budget;
                scramble(&mut value, &mut rng, &mut budget);
                text = serde_json::to_string(&value).expect("a scrambled value serializes");
            }
            if let Some((key, raw)) = dup {
                // Insert `"key":<raw>,` right after the opening brace — a
                // duplicate field wherever the key already appears first.
                text = format!("{{\"{key}\":{raw},{}", &text[1..]);
            }
            if let Some(frac) = trunc {
                let keep = (frac * text.chars().count() as f64) as usize;
                text = text.chars().take(keep).collect();
            }
            text
        })
        .boxed()
}

/// The shared assertions: no panic (proptest fails on one), and an `Ok`
/// bundle round-trips to the same hash with `to_json()` a fixpoint.
fn assert_load_contract(text: &str) -> Result<(), TestCaseError> {
    match Program::from_json(text) {
        Ok(program) => {
            let json = program.to_json();
            let reloaded = Program::from_json(&json)
                .expect("a bundle that loaded once must load from its own to_json");
            prop_assert_eq!(reloaded.hash, program.hash, "hash unstable on reload");
            prop_assert_eq!(
                reloaded.to_json(),
                json,
                "to_json is not a fixpoint for a loaded bundle"
            );
        }
        Err(err) => {
            // Typed errors only (Json / UnsupportedVersion / HashMismatch / InvalidDocument); Display is
            // part of the error contract, so exercise it too.
            let _ = err.to_string();
        }
    }
    Ok(())
}

proptest! {
    #![proptest_config(config())]

    /// Contract 1a: arbitrary bytes (lossy-decoded) and arbitrary Unicode
    /// strings never panic the loader.
    #[test]
    fn from_json_never_panics_on_arbitrary_input(
        bytes in proptest::collection::vec(any::<u8>(), 0..=300),
        text in any::<String>(),
    ) {
        assert_load_contract(&String::from_utf8_lossy(&bytes))?;
        assert_load_contract(&text)?;
    }

    /// Contract 1b/2: structured near-valid mutations of a real bundle never
    /// panic the loader, and a surviving bundle round-trips.
    #[test]
    fn mutated_bundles_load_typed_or_round_trip(text in mutated_bundle()) {
        assert_load_contract(&text)?;
    }

    /// The unscrambled base with ONLY a truncation or a duplicated key is the
    /// near-miss case loaders actually meet in the field (a torn write, a
    /// hand edit) — called out as its own property so a regression here isn't
    /// masked by the full scramble above.
    #[test]
    fn torn_or_hand_edited_bundles_behave(
        dup in proptest::option::of(proptest::sample::select(DUP_KEYS)),
        trunc in proptest::option::of(0.0f64..1.0),
    ) {
        let mut text = base_bundle().to_string();
        if let Some(key) = dup {
            text = format!("{{\"{key}\":0,{}", &text[1..]);
        }
        if let Some(frac) = trunc {
            let keep = (frac * text.chars().count() as f64) as usize;
            text = text.chars().take(keep).collect();
        }
        assert_load_contract(&text)?;
    }
}
