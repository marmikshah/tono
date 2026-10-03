//! The Rust ↔ Python cross-language contract (ADR 0004): the same song, built
//! through the Rust API here and through the typed Python API in
//! `crates/tono-py/tests/test_typed.py`, compiles to the same canonical
//! Program hash. If the pinned value changes, change BOTH sides or the
//! languages disagree.

use tono_core::catalog::{Bass, Drums};
use tono_core::prelude::*;

/// The reference song, mirrored line-for-line by the Python test.
fn reference_program() -> Program {
    let mut song = Song::new("night-drive", 122.0);
    song.add_voice("bass", &Bass::finger());
    song.add_voice("drums", &Drums::tr808());
    song.add_pattern(
        "pattern_1",
        1,
        vec![
            note(0, 2, "C2"),
            note(2, 2, "C2"),
            note(4, 2, "Eb2"),
            note(6, 2, "G2"),
        ],
    );
    song.add_pattern(
        "pattern_2",
        1,
        vec![
            note(0, 1, "midi:36"),
            note(8, 1, "midi:36"),
            note(4, 1, "midi:38"),
            note(12, 1, "midi:38"),
        ],
    );
    for bar in 0..4 {
        song.arrange("bass", "pattern_1", bar);
    }
    for bar in 0..4 {
        song.arrange("drums", "pattern_2", bar);
    }
    song.compile(&CompileOptions {
        sample_rate: Some(48_000),
        ..CompileOptions::default()
    })
    .unwrap()
}

#[test]
fn the_reference_song_pins_the_cross_language_hash() {
    let program = reference_program();
    // Cross-language contract — crates/tono-py/tests/test_typed.py builds the
    // same song via the typed Python API and pins the same hash; if this
    // value changes, change both or the languages disagree.
    assert_eq!(program.hash, 0x11EE_57F2_CF7C_7447_u64);
}

#[test]
fn the_reference_program_renders_deterministically() {
    let program = reference_program();
    let a = program.render_mono();
    assert!(!a.is_empty(), "the reference song renders audio");
    assert_eq!(a, program.render_mono(), "byte-identical every render");
}
