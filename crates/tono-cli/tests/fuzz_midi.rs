//! Property-based fuzzing of the MIDI importers —
//! the contract pinned here:
//!
//!   1. `midi::import_midi` / `midi::import_midi_song` NEVER panic — on
//!      arbitrary bytes and on structured near-valid mutations of a real
//!      exported file (bytes flipped, the file truncated, garbage appended).
//!      Whatever midly's parser accepts, the note decoding, grid quantizing,
//!      and voice mapping must survive.
//!   2. Anything they DO accept is sound: an imported `SoundDoc` validates,
//!      and an imported `Song` compiles through `to_doc` (the importers
//!      promise renderable output — `import_midi` already validates
//!      internally, so this also pins that the internal check stays honest).
//!
//! The base file is one `export_song_midi` of a two-track song (melody +
//! channel-10 kit), written once; mutations are cheap byte ops. Case budget
//! 64, matching the repo's frugal test times.

use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

use proptest::prelude::*;
use tono::midi::{export_song_midi, import_midi, import_midi_song};
use tono_core::dsl::{Adsr, SeqWave};
use tono_core::song::{Song, note};

fn config() -> ProptestConfig {
    ProptestConfig {
        cases: 64,
        // Keep the repo tree clean: failures print the seed instead of
        // writing a proptest-regressions file.
        failure_persistence: None,
        ..ProptestConfig::default()
    }
}

/// A unique temp path per case (proptest cases share the process).
fn temp_path() -> PathBuf {
    static COUNTER: AtomicUsize = AtomicUsize::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!("tono-fuzz-midi-{}-{n}.mid", std::process::id()))
}

/// A real exported SMF, built once through the public Song + export API.
fn base_midi() -> &'static [u8] {
    static BASE: std::sync::OnceLock<Vec<u8>> = std::sync::OnceLock::new();
    BASE.get_or_init(|| {
        let amp = Adsr {
            a: 0.005,
            d: 0.1,
            s: 0.8,
            r: 0.2,
            punch: 0.0,
        };
        let mut song = Song::new("fuzz-midi", 120.0);
        song.add_track("keys", SeqWave::Square, amp);
        song.tracks[0].notes = vec![
            note(0, 2, "midi:60"),
            note(2, 2, "midi:64"),
            note(4, 4, "midi:67"),
        ];
        song.add_track("drums", SeqWave::Kit, amp);
        song.tracks[1].notes = vec![note(0, 2, "midi:36"), note(4, 2, "midi:38")];
        let path = temp_path();
        export_song_midi(&song, &path).expect("the base song exports");
        let bytes = std::fs::read(&path).expect("the exported file reads back");
        let _ = std::fs::remove_file(&path);
        bytes
    })
}

/// Structured near-valid mutations of the real file: byte flips, one
/// truncation, and a garbage suffix, in any combination.
fn mutated_midi() -> BoxedStrategy<Vec<u8>> {
    (
        proptest::collection::vec((any::<proptest::sample::Index>(), any::<u8>()), 0..=16),
        proptest::option::of(any::<proptest::sample::Index>()),
        proptest::collection::vec(any::<u8>(), 0..=16),
    )
        .prop_map(|(flips, trunc, suffix)| {
            let mut bytes = base_midi().to_vec();
            for (at, mask) in flips {
                let len = bytes.len();
                bytes[at.index(len)] ^= mask;
            }
            if let Some(at) = trunc {
                bytes.truncate(at.index(bytes.len()));
            }
            bytes.extend_from_slice(&suffix);
            bytes
        })
        .boxed()
}

/// The shared assertions over both importers: no panic (proptest fails on
/// one), and accepted output validates/compiles.
fn assert_import_contract(bytes: &[u8], spb: u32) -> Result<(), TestCaseError> {
    let path = temp_path();
    std::fs::write(&path, bytes).expect("temp file writes");
    if let Ok((doc, _summary)) = import_midi(&path, spb) {
        prop_assert!(
            doc.validate().is_ok(),
            "import_midi accepted bytes but produced an invalid SoundDoc"
        );
    }
    if let Ok(song) = import_midi_song(&path, spb) {
        prop_assert!(
            song.to_doc().is_ok(),
            "import_midi_song accepted bytes but the Song doesn't compile: {:?}",
            song.to_doc().err()
        );
    }
    let _ = std::fs::remove_file(&path);
    Ok(())
}

proptest! {
    #![proptest_config(config())]

    /// Contract 1a/2: arbitrary bytes never panic the importers, and anything
    /// accepted validates.
    #[test]
    fn import_never_panics_on_arbitrary_bytes(
        bytes in proptest::collection::vec(any::<u8>(), 0..=400),
        spb in proptest::sample::select(vec![1u32, 4, 7]),
    ) {
        assert_import_contract(&bytes, spb)?;
    }

    /// Contract 1b/2: byte-flipped, truncated, or garbage-extended near-MIDI
    /// files never panic the importers, and anything accepted validates.
    #[test]
    fn import_survives_mutated_files(
        bytes in mutated_midi(),
        spb in proptest::sample::select(vec![1u32, 4, 7]),
    ) {
        assert_import_contract(&bytes, spb)?;
    }
}

#[test]
fn unsupported_timecode_headers_never_panic() {
    for division in [0x80, 0xE7, 0xE8, 0xE2] {
        let mut bytes = base_midi().to_vec();
        bytes[12] = division;
        assert_import_contract(&bytes, 4).unwrap();
        let mut riff = b"RIFF".to_vec();
        riff.extend_from_slice(&(12 + bytes.len() as u32).to_le_bytes());
        riff.extend_from_slice(b"RMIDdata");
        riff.extend_from_slice(&(bytes.len() as u32).to_le_bytes());
        riff.extend_from_slice(&bytes);
        assert_import_contract(&riff, 4).unwrap();
    }
}
