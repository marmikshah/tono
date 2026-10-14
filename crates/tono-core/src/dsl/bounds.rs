//! The numeric bounds [`SoundDoc::validate`](super::SoundDoc::validate)
//! enforces, and the tempo-map rule the seq validator, the song compiler and
//! the bindings share. One place: `vary::mutate` clamps into these, the
//! renderer's defensive clamps quote them, and authoring tools can show them.

use std::ops::RangeInclusive;

use super::TempoPoint;
use crate::units::Beat;

/// Longest document in seconds; the cap bounds render memory.
pub const MAX_DURATION_SECS: f32 = 600.0;
/// Supported sample rates in Hz.
pub const SAMPLE_RATE_HZ: RangeInclusive<u32> = 8_000..=192_000;
/// Haas inter-channel delay in milliseconds.
pub const HAAS_MS: RangeInclusive<f32> = 0.5..=40.0;
/// Loudness-normalization target in LUFS.
pub const TARGET_LUFS: RangeInclusive<f32> = -60.0..=0.0;
/// True-peak ceiling in dBTP.
pub const CEILING_DBTP: RangeInclusive<f32> = -12.0..=0.0;
/// Stereo position of a layer or send.
pub const PAN: RangeInclusive<f32> = -1.0..=1.0;
/// A channel or bus fader (1 = unity).
pub const FADER_GAIN: RangeInclusive<f32> = 0.0..=2.0;
/// Highest constant frequency in Hz: above every supported Nyquist, and far
/// below the products (`fm.freq × fm.ratio`) that overflow f32.
pub const MAX_CONST_HZ: f32 = 100_000.0;
/// Largest magnitude of a modulated frequency's endpoints in Hz.
pub const MAX_MOD_HZ: f32 = 1e6;
/// Fastest `rand` walk in targets per second; past it the per-sample
/// catch-up loop becomes a denial of service.
pub const MAX_RAND_RATE: f32 = 10_000.0;
/// Highest FM modulator ratio, so `freq × ratio` stays finite.
pub const MAX_FM_RATIO: f32 = 4096.0;
/// Highest seq FM index.
pub const MAX_FM_INDEX: f32 = 20.0;
/// Unison voice count of a `super` oscillator.
pub const SUPER_VOICES: RangeInclusive<u32> = 1..=16;
/// Widest unison spread in cents (10 octaves); past it `2^(cents/1200)`
/// approaches f32 overflow.
pub const MAX_DETUNE_CENTS: f32 = 12_000.0;
/// EQ boost/cut magnitude in dB; far beyond it the biquad coefficients
/// overflow.
pub const EQ_GAIN_DB: f32 = 24.0;
/// Longest delay line in seconds; the cap bounds the allocation.
pub const MAX_DELAY_SECS: f32 = 30.0;
/// Fastest tremolo in Hz; past it the wobble reads as a tone.
pub const MAX_TREMOLO_RATE_HZ: f32 = 40.0;
/// Most resonators in a `modal` bank.
pub const MAX_MODAL_MODES: usize = 64;
/// Deepest graph nesting; keeps validation and the evaluator off the stack.
pub const MAX_NODE_DEPTH: usize = 256;
/// Most points in a tempo map.
pub const MAX_TEMPO_POINTS: usize = 1024;
/// Most points in a song's meter map.
pub const MAX_METER_POINTS: usize = 256;
/// Largest time-signature denominator (a power of two).
pub const MAX_METER_DENOMINATOR: u32 = 64;
/// Highest General MIDI program number.
pub const MAX_SF2_PRESET: u32 = 127;

/// One problem with a tempo or meter map: where it is (a path like
/// `tempo_map[2].bpm`), what is wrong, and how to fix it. The seq validator
/// reports the first; the song compiler reports them all as diagnostics.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MapProblem {
    /// The offending field, relative to the map's owner.
    pub path: String,
    /// What is wrong.
    pub message: String,
    /// How to fix it.
    pub remediation: &'static str,
}

/// Every problem with a tempo map: bounded length, first point at beat 0,
/// positive finite tempos, strictly ascending beats. An empty map (constant
/// tempo) has none.
pub fn tempo_map_problems(map: &[TempoPoint]) -> Vec<MapProblem> {
    let mut out = Vec::new();
    let mut push = |path: String, message: String, remediation: &'static str| {
        out.push(MapProblem {
            path,
            message,
            remediation,
        })
    };
    if map.len() > MAX_TEMPO_POINTS {
        push(
            "tempo_map".into(),
            format!(
                "tempo_map is capped at {MAX_TEMPO_POINTS} points, got {}",
                map.len()
            ),
            "split the piece or thin the tempo changes",
        );
    }
    if let Some(first) = map.first()
        && first.at != Beat::zero()
    {
        push(
            "tempo_map[0].at".into(),
            "tempo_map's first point must be at beat 0".into(),
            "add a point at beat 0 (the song's bpm applies before the first change otherwise)",
        );
    }
    for (i, p) in map.iter().enumerate() {
        if !(p.bpm.is_finite() && p.bpm > 0.0) {
            push(
                format!("tempo_map[{i}].bpm"),
                format!("tempo must be positive and finite, got {}", p.bpm),
                "set a tempo above 0 BPM",
            );
        }
        if i > 0 && p.at <= map[i - 1].at {
            push(
                format!("tempo_map[{i}].at"),
                "tempo_map must be strictly ascending by beat".into(),
                "sort the changes and merge any duplicates",
            );
        }
    }
    out
}
