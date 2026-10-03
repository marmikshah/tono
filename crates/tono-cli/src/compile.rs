//! `tono compile` — compile a Song (JSON) into a validated, hashed
//! [`Program`] bundle, with machine-readable inspection output.
//!
//! The shell (`src/main.rs`) owns argument parsing and file output; the
//! substance lives here and stays testable.

use std::fs;

use anyhow::Context;
use tono_core::program::Program;
use tono_core::song::{CompileOptions, CompileTarget, Song};

/// Parse a song file and compile it. Every compile problem arrives in one
/// pass as a multi-line error whose diagnostics carry stable codes and fixes.
pub fn compile_song(file: &str, sample_rate: Option<u32>) -> anyhow::Result<Program> {
    let text = fs::read_to_string(file).with_context(|| format!("reading {file}"))?;
    let song: Song =
        serde_json::from_str(&text).with_context(|| format!("parsing {file} as a Song"))?;
    let options = CompileOptions {
        sample_rate,
        target: CompileTarget::Offline,
    };
    Ok(song.compile(&options)?)
}

/// The machine-readable program summary `--inspect` prints: identity, version
/// pins, the canonical hash (an exact JSON integer — u64 text, no float
/// rounding), the track roster, the resource estimates, and the warnings.
pub fn inspect_json(program: &Program) -> serde_json::Value {
    let meta = &program.meta;
    serde_json::json!({
        "name": program.doc.name,
        "hash": program.hash,
        "program_version": program.program_version,
        "schema_version": program.doc.version,
        "engine_version": program.doc.engine,
        "target": program.target,
        "capabilities": program.capabilities(),
        "tempo_bpm": meta.tempo_bpm,
        "beats_per_bar": meta.beats_per_bar,
        "steps_per_beat": meta.steps_per_beat,
        "length_bars": meta.length_bars,
        "duration_seconds": program.doc.duration,
        "duration_frames": program.estimates.frames,
        "sample_rate": program.doc.sample_rate,
        "streamable": program.is_streamable(),
        "tracks": meta.tracks.iter().enumerate().map(|(index, t)| serde_json::json!({
            "index": index,
            "name": t.name,
            "wave": t.wave,
            "notes": t.notes,
            "mute": t.mute,
            "solo": t.solo,
        })).collect::<Vec<_>>(),
        "estimates": {
            "frames": program.estimates.frames,
            "events": program.estimates.events,
            "peak_voices": program.estimates.peak_voices,
            "memory_bytes": program.estimates.memory_bytes,
        },
        "warnings": program.warnings(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use tono_core::dsl::{Adsr, SeqWave};
    use tono_core::song::note;

    fn amp() -> Adsr {
        Adsr {
            a: 0.005,
            d: 0.1,
            s: 0.8,
            r: 0.2,
            punch: 0.0,
        }
    }

    fn demo_song() -> Song {
        let mut song = Song::new("demo", 120.0);
        song.add_track("bass", SeqWave::Bass, amp());
        song.add_pattern("riff", 1, vec![note(0, 4, "C2")]);
        song.arrange("bass", "riff", 0);
        song
    }

    fn write_temp(name: &str, contents: &str) -> String {
        let path = std::env::temp_dir().join(name);
        fs::write(&path, contents).expect("write temp song");
        path.to_string_lossy().into_owned()
    }

    #[test]
    fn compiles_a_song_file() {
        let path = write_temp(
            "tono_test_compile_song.json",
            &serde_json::to_string(&demo_song()).unwrap(),
        );
        let program = compile_song(&path, Some(48_000)).expect("compiles");
        assert_eq!(program.doc.name, "demo");
        assert_eq!(program.doc.sample_rate, 48_000);
        assert_eq!(program.meta.tracks.len(), 1);
        assert_eq!(program.doc.version, tono_core::dsl::SCHEMA_VERSION);
    }

    #[test]
    fn compile_failures_carry_every_diagnostic() {
        let mut song = demo_song();
        song.arrange("nope", "riff", 0);
        song.arrange("bass", "ghost", 1);
        let path = write_temp(
            "tono_test_compile_bad_song.json",
            &serde_json::to_string(&song).unwrap(),
        );
        let err = compile_song(&path, None).unwrap_err();
        let text = format!("{err}");
        assert!(text.contains("T1001"), "unknown track: {text}");
        assert!(text.contains("T1002"), "unknown pattern: {text}");
        assert!(text.contains("arrangement[1].track"), "the path: {text}");
        assert!(text.contains("arrangement[2].pattern"), "the path: {text}");
    }

    #[test]
    fn inspect_is_machine_readable_and_exact() {
        let program = demo_song()
            .compile(&CompileOptions::default())
            .expect("compiles");
        let inspect = inspect_json(&program);
        assert_eq!(inspect["name"], "demo");
        // The hash is an exact JSON integer (no f64 rounding of the u64).
        assert_eq!(inspect["hash"].as_u64().unwrap(), program.hash);
        assert_eq!(
            inspect["program_version"],
            tono_core::program::PROGRAM_VERSION
        );
        assert_eq!(inspect["tracks"][0]["name"], "bass");
        assert_eq!(inspect["tracks"][0]["index"], 0);
        assert_eq!(inspect["estimates"]["events"], 1);
        // A plain compiled song (schema v2, built-in waves) streams natively.
        assert_eq!(
            inspect["streamable"].as_bool(),
            Some(true),
            "the demo song is streamable: {inspect}"
        );
        assert_eq!(
            inspect["warnings"].as_array().unwrap().len(),
            0,
            "no streaming blockers: {inspect}"
        );
        // Round-trips through a JSON parser losslessly (the CLI prints this).
        let reparsed: serde_json::Value =
            serde_json::from_str(&serde_json::to_string_pretty(&inspect).unwrap()).unwrap();
        assert_eq!(reparsed["hash"].as_u64().unwrap(), program.hash);
    }
}
