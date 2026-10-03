//! pattern — pure transforms on a [`Pattern`]: rhythm surgery that produces
//! NEW patterns and never mutates its input.
//!
//! Every function takes and returns the song's [`Pattern`] (`name` + `bars` +
//! notes on the step grid); nothing here touches the audio path — transforms
//! compose freely, and the result drops back into
//! [`Song::add_pattern`](super::Song::add_pattern) like any authored phrase.
//! Ops that need the grid (`repeat`, `concat`, `slice`, `rotate`, `reverse`)
//! take `steps_per_bar` explicitly — the song's `beats_per_bar ×
//! steps_per_beat`, floored at 1 like the song compiler — so a pattern stays
//! a self-contained value with no ambient context.
//!
//! Naming: every transform names its result `{input}_{suffix}` —
//! [`repeat`] → `riff_x2`, [`reverse`] → `riff_rev`, [`transpose`] →
//! `riff_t+5` — so a derived pattern reads as derived in the arrangement.
//! The name is a label, not data: rename before `add_pattern` when it
//! matters. The generators ([`euclidean`], [`tuplet`]) take an explicit name
//! instead — they mint a pattern from nothing.
//!
//! Determinism: [`probability`] and [`humanize`] draw per note from
//! `Rng::new(seed ^ note_identity)`, where the identity mixes (step, len,
//! pitch) exactly like the seq renderer's humanize salts — so the same
//! pattern + the same seed always gives the same result, and a note's draw
//! follows the NOTE, never its position in the vec or the wall clock.
//!
//! ```
//! use tono_core::dsl::Value;
//! use tono_core::song::{Pattern, euclidean, note, repeat, transpose};
//!
//! let riff = Pattern {
//!     name: "riff".into(),
//!     bars: 1,
//!     notes: vec![note(0, 4, "C2"), note(8, 4, "G2")],
//! };
//! let doubled = repeat(&riff, 16, 2);
//! assert_eq!(doubled.bars, 2);
//! assert_eq!(doubled.name, "riff_x2");
//!
//! let clave = euclidean("clave", 3, 8, "midi:36", 2, 16).unwrap();
//! assert_eq!(
//!     clave.notes.iter().map(|n| n.step).collect::<Vec<_>>(),
//!     [0, 3, 6]
//! );
//!
//! let up = transpose(&riff, 2).unwrap();
//! let Value::Note(name) = &up.notes[0].pitch else {
//!     panic!("a note name stays a note name")
//! };
//! assert_eq!(name, "D2");
//! ```
//!

use super::Pattern;
use crate::dsl::{SeqNote, Value};
use crate::dsp::Rng;
use crate::music::{MusicError, Pitch};

/// The pattern's total length in steps (`bars × steps_per_bar`), in u64 so a
/// pathological grid can't wrap the arithmetic mid-op.
fn total_steps(p: &Pattern, steps_per_bar: u32) -> u64 {
    p.bars as u64 * steps_per_bar.max(1) as u64
}

/// A short pitch label for error messages: the name, the Hz, or "modulated".
fn pitch_label(v: &Value) -> String {
    match v {
        Value::Note(s) => format!("\"{s}\""),
        Value::Const(hz) => format!("{hz} Hz"),
        Value::Modulated(_) => "a modulated pitch".into(),
    }
}

/// The seed salt shared with the seq renderer's per-note humanize stream
/// (`render/seq.rs`, frac(√2)·2³²) — the draws here stay in the same
/// deterministic family as the render-time jitter, pinned to the same value.
const PATTERN_SEED_SALT: u64 = 0x6A09_E667;

/// A note's stable identity for the deterministic ops: the same (step, len,
/// pitch) mix the seq renderer's humanize uses, so a note's draw follows the
/// note itself. Duplicated notes (same step, len, AND pitch) share a draw —
/// they jitter, keep, and drop together.
fn note_identity(n: &SeqNote) -> u64 {
    let mut id = (n.step as u64) << 32 ^ (n.len as u64) << 8 ^ PATTERN_SEED_SALT;
    id ^= pitch_identity(&n.pitch).rotate_left(17);
    id
}

/// The pitch's contribution to a note identity — the local copy of
/// `render/seq.rs`'s private `pitch_identity` (kept private there; the two
/// must not drift, so both hash through the same primitives).
fn pitch_identity(v: &Value) -> u64 {
    match v {
        Value::Const(c) => c.to_bits() as u64,
        Value::Note(s) => crate::dsp::layer_stream_key(s),
        // A per-note modulated pitch is already unique enough by its step/len
        // in practice; all share one tag.
        Value::Modulated(_) => 0x4D4F_4455,
    }
}

/// The pattern concatenated with itself `times` times: `bars × times`, with
/// copy `i` offset by `i × steps_per_bar × bars`. `times` 0 gives an empty,
/// zero-bar pattern (a deliberate silence — [`Song::add_pattern`](
/// super::Song::add_pattern) floors the bar count back to 1). Name suffix
/// `_x{times}`: `riff` × 2 → `riff_x2`.
pub fn repeat(p: &Pattern, steps_per_bar: u32, times: u32) -> Pattern {
    let stride = total_steps(p, steps_per_bar);
    let mut notes = Vec::with_capacity(p.notes.len().saturating_mul(times as usize));
    for i in 0..times as u64 {
        let off = i * stride;
        for n in &p.notes {
            let mut m = n.clone();
            m.step = (n.step as u64 + off).min(u32::MAX as u64) as u32;
            notes.push(m);
        }
    }
    Pattern {
        name: format!("{}_x{times}", p.name),
        bars: p.bars.saturating_mul(times),
        notes,
    }
}

/// `a` then `b` back-to-back: `b`'s notes offset by `a.bars × steps_per_bar`,
/// `bars = a.bars + b.bars`. Name suffix is the appended pattern's name:
/// `verse` + `chorus` → `verse_chorus`.
pub fn concat(a: &Pattern, b: &Pattern, steps_per_bar: u32) -> Pattern {
    let off = a.bars as u64 * steps_per_bar.max(1) as u64;
    let mut notes = a.notes.clone();
    notes.extend(b.notes.iter().map(|n| {
        let mut m = n.clone();
        m.step = (n.step as u64 + off).min(u32::MAX as u64) as u32;
        m
    }));
    Pattern {
        name: format!("{}_{}", a.name, b.name),
        bars: a.bars.saturating_add(b.bars),
        notes,
    }
}

/// Both patterns played at once: the note sets merged and sorted by step —
/// the sort is stable and `a`'s notes come first, so on a tie `a` leads
/// (its hit is the one a listener hears as "the" downbeat). `bars` is the
/// longer of the two. Name: `a_layer_b`.
pub fn layer(a: &Pattern, b: &Pattern) -> Pattern {
    let mut notes = a.notes.clone();
    notes.extend(b.notes.iter().cloned());
    notes.sort_by_key(|n| n.step);
    Pattern {
        name: format!("{}_layer_{}", a.name, b.name),
        bars: a.bars.max(b.bars),
        notes,
    }
}

/// The window `[start_step, start_step + len_steps)` of the pattern: notes
/// STARTING inside it are kept and re-based to step 0. A kept note's tail may
/// overrun the slice end — slicing cuts on note STARTS, never mid-note. `bars
/// = ceil(len_steps / steps_per_bar)`, at least 1. Name suffix
/// `_slice_{start}_{len}`.
pub fn slice(p: &Pattern, start_step: u32, len_steps: u32, steps_per_bar: u32) -> Pattern {
    let end = start_step as u64 + len_steps as u64;
    let notes = p
        .notes
        .iter()
        .filter(|n| start_step as u64 <= n.step as u64 && (n.step as u64) < end)
        .map(|n| {
            let mut m = n.clone();
            m.step -= start_step;
            m
        })
        .collect();
    let bars = (len_steps as u64)
        .div_ceil(steps_per_bar.max(1) as u64)
        .clamp(1, u32::MAX as u64) as u32;
    Pattern {
        name: format!("{}_slice_{start_step}_{len_steps}", p.name),
        bars,
        notes,
    }
}

/// Every pitch shifted by `semitones` (negative descends). Note names go
/// through [`Pitch`]'s strict grammar and come back in canonical sharp
/// spelling (`"Gb3"` transposed by 0 is `"F#3"`); `"midi:N"` pitches shift
/// the integer and stay in `"midi:N"` form — a kit note stays a kit note.
/// `Value::Const` Hz scales by 2^(semitones/12). A `Value::Modulated` pitch
/// is left untouched (transposing a glide is a design decision, not a
/// transform — reshape the modulator by hand).
///
/// Errors loudly, never skips: an unparseable name fails with
/// [`PatternError::Pitch`] (the music module's own error), a note pushed
/// outside 0..=127 with [`PatternError::BadTranspose`] naming it. Name suffix
/// `_t{semitones:+}`: `riff_t+5`, `riff_t-12`.
pub fn transpose(p: &Pattern, semitones: i16) -> Result<Pattern, PatternError> {
    let factor = 2f32.powf(semitones as f32 / 12.0);
    let mut notes = Vec::with_capacity(p.notes.len());
    for n in &p.notes {
        let mut m = n.clone();
        m.pitch = match &n.pitch {
            Value::Note(s) => {
                let midi_form = s.starts_with("midi:");
                let pitch = Pitch::from_name(s)?;
                let shifted = pitch.add_semitones(semitones).map_err(|_| {
                    PatternError::BadTranspose(format!(
                        "transposing the note at step {} ({}) by {semitones:+} semitones leaves \
                         the MIDI range 0..=127 — transpose fewer semitones, or drop or rewrite \
                         that note first",
                        n.step,
                        pitch_label(&n.pitch),
                    ))
                })?;
                if midi_form {
                    Value::Note(format!("midi:{}", shifted.to_midi()))
                } else {
                    Value::Note(shifted.to_string())
                }
            }
            Value::Const(hz) => {
                let scaled = hz * factor;
                if !scaled.is_finite() || scaled <= 0.0 {
                    return Err(PatternError::BadTranspose(format!(
                        "transposing the note at step {} ({}) by {semitones:+} semitones \
                         overflowed: a Hz constant scales by 2^(semitones/12) — use fewer \
                         semitones, or rewrite the pitch as a note name",
                        n.step,
                        pitch_label(&n.pitch),
                    )));
                }
                Value::Const(scaled)
            }
            Value::Modulated(_) => n.pitch.clone(),
        };
        notes.push(m);
    }
    Ok(Pattern {
        name: format!("{}_t{semitones:+}", p.name),
        bars: p.bars,
        notes,
    })
}

/// Time scaled by exactly `num/den`: every step and len multiplied by `num`
/// and divided by `den` — and EXACTLY means exactly. If any note (or the bar
/// count, `bars × num / den`) would land between grid steps the whole stretch
/// fails with [`PatternError::OffGrid`] naming the offending note; nothing is
/// ever rounded silently. Stretch by 2/1 to double time, 3/2 to play the
/// phrase as a hemiola. A degenerate ratio (either part 0) is
/// [`PatternError::BadParams`]. `steps_per_bar` rides along for a uniform
/// call shape with the other grid ops — the exactness rule needs only the
/// ratio. Name suffix `_stretch_{num}_{den}`.
pub fn stretch(
    p: &Pattern,
    num: u32,
    den: u32,
    steps_per_bar: u32,
) -> Result<Pattern, PatternError> {
    let _ = steps_per_bar;
    if num == 0 || den == 0 {
        return Err(PatternError::BadParams(format!(
            "stretch ratio {num}/{den} is degenerate — numerator and denominator must both be \
             ≥ 1 (2/1 doubles time, 1/2 halves it)"
        )));
    }
    let (num64, den64) = (num as u64, den as u64);
    let scaled_bars = p.bars as u64 * num64;
    if !scaled_bars.is_multiple_of(den64) {
        return Err(PatternError::OffGrid {
            what: "the bar count".into(),
            value: p.bars,
            num,
            den,
        });
    }
    let mut notes = Vec::with_capacity(p.notes.len());
    for n in &p.notes {
        let step = n.step as u64 * num64;
        if !step.is_multiple_of(den64) {
            return Err(PatternError::OffGrid {
                what: format!(
                    "the note at step {} (pitch {})",
                    n.step,
                    pitch_label(&n.pitch)
                ),
                value: n.step,
                num,
                den,
            });
        }
        let len = n.len as u64 * num64;
        if !len.is_multiple_of(den64) {
            return Err(PatternError::OffGrid {
                what: format!(
                    "the length of the note at step {} (pitch {})",
                    n.step,
                    pitch_label(&n.pitch)
                ),
                value: n.len,
                num,
                den,
            });
        }
        let mut m = n.clone();
        m.step = (step / den64).min(u32::MAX as u64) as u32;
        m.len = (len / den64).min(u32::MAX as u64) as u32;
        notes.push(m);
    }
    Ok(Pattern {
        name: format!("{}_stretch_{num}_{den}", p.name),
        bars: (scaled_bars / den64).min(u32::MAX as u64) as u32,
        notes,
    })
}

/// Every note's start moved by `shift_steps` within the pattern's total
/// length, WRAPPING — a true rotate, `(step + shift) mod total`, so a note
/// pushed past the end comes back in at the top. Only the starts wrap; a
/// wrapped note's tail may poke past the pattern end (the seq renderer caps
/// notes at the render window anyway). Notes come out sorted by step. Name
/// suffix `_rot{shift:+}`.
pub fn rotate(p: &Pattern, shift_steps: i64, steps_per_bar: u32) -> Pattern {
    let total = total_steps(p, steps_per_bar) as i128;
    let mut notes: Vec<SeqNote> = p
        .notes
        .iter()
        .map(|n| {
            let mut m = n.clone();
            if total > 0 {
                m.step = (n.step as i128 + shift_steps as i128).rem_euclid(total) as u32;
            }
            m
        })
        .collect();
    notes.sort_by_key(|n| n.step);
    Pattern {
        name: format!("{}_rot{shift_steps:+}", p.name),
        bars: p.bars,
        notes,
    }
}

/// The pattern mirrored in time: a note occupying `[s, s + len)` maps to
/// `[total − s − len, total − s)`, so what answered now calls. A note poking
/// past the pattern end mirrors onto step 0 (saturating — nothing wraps).
/// Notes come out sorted by step. Name suffix `_rev`.
pub fn reverse(p: &Pattern, steps_per_bar: u32) -> Pattern {
    let total = total_steps(p, steps_per_bar);
    let mut notes: Vec<SeqNote> = p
        .notes
        .iter()
        .map(|n| {
            let mut m = n.clone();
            m.step = total
                .saturating_sub(n.step as u64 + n.len as u64)
                .min(u32::MAX as u64) as u32;
            m
        })
        .collect();
    notes.sort_by_key(|n| n.step);
    Pattern {
        name: format!("{}_rev", p.name),
        bars: p.bars,
        notes,
    }
}

/// Note starts snapped to the nearest multiple of `grid_steps` (halves round
/// away from zero, so a 2 on a 4-grid snaps forward to 4); lengths are
/// unchanged — quantize places attacks, it doesn't rephrase. `grid_steps` is
/// floored at 1 (a no-op grid). Notes come out sorted by step. Name suffix
/// `_q{grid_steps}`.
pub fn quantize(p: &Pattern, grid_steps: u32) -> Pattern {
    let grid = grid_steps.max(1) as u64;
    let mut notes: Vec<SeqNote> = p
        .notes
        .iter()
        .map(|n| {
            let mut m = n.clone();
            let s = n.step as u64;
            m.step = (((s + grid / 2) / grid) * grid).min(u32::MAX as u64) as u32;
            m
        })
        .collect();
    notes.sort_by_key(|n| n.step);
    Pattern {
        name: format!("{}_q{grid_steps}", p.name),
        bars: p.bars,
        notes,
    }
}

/// Every gain multiplied by `scale`, clamped to the seq's 0..1 velocity
/// convention — `vel(&p, 0.7)` to sit a part back in the mix. Name suffix
/// `_vel{scale}`.
pub fn vel(p: &Pattern, scale: f32) -> Pattern {
    let notes = p
        .notes
        .iter()
        .map(|n| {
            let mut m = n.clone();
            m.gain = (n.gain * scale).clamp(0.0, 1.0);
            m
        })
        .collect();
    Pattern {
        name: format!("{}_vel{scale}", p.name),
        bars: p.bars,
        notes,
    }
}

/// Every length multiplied by `factor`, rounded to the nearest step and
/// floored at 1 — a note never vanishes. 0.5 is the classic staccato tighten,
/// 2.0 doubles the sustain. Name suffix `_gate{factor}`.
pub fn gate(p: &Pattern, factor: f32) -> Pattern {
    let notes = p
        .notes
        .iter()
        .map(|n| {
            let mut m = n.clone();
            m.len = ((n.len as f32 * factor).round() as u32).max(1);
            m
        })
        .collect();
    Pattern {
        name: format!("{}_gate{factor}", p.name),
        bars: p.bars,
        notes,
    }
}

/// Deterministic per-note keep/drop: a note survives when its draw from
/// `Rng::new(seed ^ note_identity)` falls under `keep` (clamped to 0..1).
/// Same pattern + same seed ⇒ same drops — guaranteed: the draw depends only
/// on (seed, step, len, pitch), never on vec order, so reordering the notes
/// changes nothing and two identical notes drop or keep together. `keep` 1.0
/// keeps everything, 0.0 drops everything. Name suffix `_prob{keep}`.
pub fn probability(p: &Pattern, keep: f32, seed: u64) -> Pattern {
    let keep = keep.clamp(0.0, 1.0);
    let notes = p
        .notes
        .iter()
        .filter(|n| Rng::new(seed ^ note_identity(n)).unit() < keep)
        .cloned()
        .collect();
    Pattern {
        name: format!("{}_prob{keep}", p.name),
        bars: p.bars,
        notes,
    }
}

/// `pulses` hits Bresenham-evenly across `steps` grid positions — the
/// euclidean-rhythm construction: `euclidean("clave", 3, 8, ..)` places hits
/// at 0, 3, 6, the tresillo. Every hit gets `pitch` (a kit note like
/// `"midi:36"` reads naturally here) and length `len` (floored at 1), at full
/// velocity. `bars = ceil(steps / steps_per_bar)`, at least 1. Errors
/// [`PatternError::BadParams`] when there are more pulses than positions (or
/// no positions at all) — for density past 1.0, [`layer`] two patterns.
pub fn euclidean(
    name: &str,
    pulses: u32,
    steps: u32,
    pitch: &str,
    len: u32,
    steps_per_bar: u32,
) -> Result<Pattern, PatternError> {
    if steps == 0 {
        return Err(PatternError::BadParams(
            "euclidean needs at least 1 grid step — steps is the cycle length".into(),
        ));
    }
    if pulses > steps {
        return Err(PatternError::BadParams(format!(
            "euclidean({pulses} pulses, {steps} steps) packs more pulses than grid positions — \
             pulses must be ≤ steps; for denser hits, layer two patterns"
        )));
    }
    let notes = (0..steps)
        .filter(|i| (*i as u64 * pulses as u64) % (steps as u64) < pulses as u64)
        .map(|i| SeqNote {
            step: i,
            len: len.max(1),
            pitch: Value::Note(pitch.into()),
            gain: 1.0,
        })
        .collect();
    let bars = (steps as u64)
        .div_ceil(steps_per_bar.max(1) as u64)
        .clamp(1, u32::MAX as u64) as u32;
    Ok(Pattern {
        name: name.into(),
        bars,
        notes,
    })
}

/// `count` notes spaced evenly across `in_steps` steps — the triplet/quintuplet
/// constructor: position `round(i × in_steps / count)`, halves away from zero,
/// each `len_steps` long (floored at 1) at full velocity. The tuplet is
/// written for one bar (`bars` is 1 — [`repeat`] or [`stretch`] it to span
/// more). `count` 0 gives an empty pattern.
pub fn tuplet(name: &str, count: u32, in_steps: u32, pitch: &str, len_steps: u32) -> Pattern {
    let notes = (0..count)
        .map(|i| {
            // round(i × in_steps / count) with halves away from zero, in u64
            // so a pathological grid can't wrap the multiply.
            let num = 2 * i as u64 * in_steps as u64 + count as u64;
            let step = (num / (2 * count as u64)).min(u32::MAX as u64) as u32;
            SeqNote {
                step,
                len: len_steps.max(1),
                pitch: Value::Note(pitch.into()),
                gain: 1.0,
            }
        })
        .collect();
    Pattern {
        name: name.into(),
        bars: 1,
        notes,
    }
}

/// Deterministic per-note jitter, baked into the pattern: each note's step
/// shifts by `±timing` steps at `timing` 1.0 (drawn from `Rng::new(seed ^
/// note_identity)`, rounded to an integer, clamped ≥ 0) and each gain wobbles
/// by `±velocity` at `velocity` 1.0 (clamped to 0..1). Same pattern + same
/// seed ⇒ same result — the draws follow (step, len, pitch), not vec order.
/// Notes come out sorted by step. Name suffix `_hum`.
///
/// This is STRUCTURAL humanization: the jitter becomes the pattern, so it
/// exports, serializes, and renders identically under any engine settings.
/// The song/track `humanize` field applies the same idea per render instead
/// — use that while the amount is still a mix decision, and this when the
/// slop IS the part.
pub fn humanize(p: &Pattern, timing: f32, velocity: f32, seed: u64) -> Pattern {
    let timing = timing.max(0.0);
    let velocity = velocity.max(0.0);
    let mut notes: Vec<SeqNote> = p
        .notes
        .iter()
        .map(|n| {
            let mut rng = Rng::new(seed ^ note_identity(n));
            let (timing_draw, gain_draw) = (rng.bi(), rng.bi());
            let mut m = n.clone();
            let jitter = (timing_draw * timing).round() as i64;
            m.step = (n.step as i64)
                .saturating_add(jitter)
                .clamp(0, u32::MAX as i64) as u32;
            m.gain = (n.gain + gain_draw * velocity).clamp(0.0, 1.0);
            m
        })
        .collect();
    notes.sort_by_key(|n| n.step);
    Pattern {
        name: format!("{}_hum", p.name),
        bars: p.bars,
        notes,
    }
}

/// Why a pattern transform failed. Hand-rolled, like the rest of the core's
/// errors; every message names the note or parameter and suggests the fix, so
/// a tool can pattern-match and self-correct.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PatternError {
    /// A [`stretch`] would land a value between grid steps — stretch never
    /// rounds silently.
    OffGrid {
        /// Which value broke: the note (named by step and pitch) or the bar
        /// count.
        what: String,
        /// The unscaled value (the note's step or len, or the bar count).
        value: u32,
        /// The stretch numerator attempted.
        num: u32,
        /// The stretch denominator attempted.
        den: u32,
    },
    /// A [`transpose`] pushed a note outside the usable range — the message
    /// names the note, the shift, and the fix.
    BadTranspose(String),
    /// Degenerate parameters (a zero stretch ratio, more pulses than steps) —
    /// the message names the parameter and the fix.
    BadParams(String),
    /// The pitch math itself rejected a name — the [`music`](crate::music)
    /// module's own error, verbatim (its messages already name the input and
    /// the valid grammar).
    Pitch(MusicError),
}

impl std::fmt::Display for PatternError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PatternError::OffGrid {
                what,
                value,
                num,
                den,
            } => write!(
                f,
                "stretch {num}/{den} pulls {what} off the grid: {value} × {num} isn't divisible \
                 by {den} — pick a ratio that keeps every step, len, and the bar count integral, \
                 or quantize first"
            ),
            PatternError::BadTranspose(msg) | PatternError::BadParams(msg) => f.write_str(msg),
            PatternError::Pitch(e) => e.fmt(f),
        }
    }
}

impl std::error::Error for PatternError {}

impl From<MusicError> for PatternError {
    fn from(e: MusicError) -> Self {
        PatternError::Pitch(e)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dsl::{Adsr, Modulator, SeqWave};
    use crate::song::{Song, note, note_vel};

    /// The one-bar riff most tests start from: C2 on 0, G2 (softer) on 8.
    fn riff() -> Pattern {
        Pattern {
            name: "riff".into(),
            bars: 1,
            notes: vec![note(0, 4, "C2"), note_vel(8, 4, "G2", 0.8)],
        }
    }

    fn steps(p: &Pattern) -> Vec<u32> {
        p.notes.iter().map(|n| n.step).collect()
    }

    fn lens(p: &Pattern) -> Vec<u32> {
        p.notes.iter().map(|n| n.len).collect()
    }

    fn names(p: &Pattern) -> Vec<String> {
        p.notes
            .iter()
            .map(|n| match &n.pitch {
                Value::Note(s) => s.clone(),
                Value::Const(hz) => format!("{hz}Hz"),
                Value::Modulated(_) => "mod".into(),
            })
            .collect()
    }

    #[test]
    fn repeat_offsets_each_copy_by_the_pattern_length() {
        let p = repeat(&riff(), 16, 3);
        assert_eq!(p.name, "riff_x3");
        assert_eq!(p.bars, 3);
        assert_eq!(steps(&p), [0, 8, 16, 24, 32, 40]);
        assert_eq!(names(&p), ["C2", "G2", "C2", "G2", "C2", "G2"]);
        // times 0 is a deliberate silence: no notes, no bars.
        let zero = repeat(&riff(), 16, 0);
        assert_eq!(zero.name, "riff_x0");
        assert_eq!(zero.bars, 0);
        assert!(zero.notes.is_empty());
    }

    #[test]
    fn concat_appends_b_after_a() {
        let mut b = riff();
        b.name = "fill".into();
        b.bars = 2;
        b.notes = vec![note(0, 1, "C3"), note(4, 1, "D3")];
        let p = concat(&riff(), &b, 16);
        assert_eq!(p.name, "riff_fill");
        assert_eq!(p.bars, 3);
        // b's notes land one riff-bar (16 steps) in.
        assert_eq!(steps(&p), [0, 8, 16, 20]);
    }

    #[test]
    fn layer_merges_sorted_with_a_leading_on_ties() {
        let drums = Pattern {
            name: "drums".into(),
            bars: 2,
            notes: vec![note(4, 1, "midi:36"), note(8, 1, "midi:38")],
        };
        let p = layer(&riff(), &drums);
        assert_eq!(p.name, "riff_layer_drums");
        assert_eq!(p.bars, 2, "bars is the longer of the two");
        assert_eq!(steps(&p), [0, 4, 8, 8]);
        // The tie at step 8: a's G2 precedes b's snare.
        assert_eq!(names(&p), ["C2", "midi:36", "G2", "midi:38"]);
    }

    #[test]
    fn slice_rebases_and_lets_tails_overrun() {
        let p = Pattern {
            name: "line".into(),
            bars: 1,
            notes: vec![
                note(0, 2, "C4"),
                note(4, 8, "E4"),
                note(8, 8, "G4"),
                note(12, 2, "B4"),
            ],
        };
        // Window [4, 12): the notes at 4 and 8 are kept and re-based; the
        // G4's tail runs to 16 — past the slice end, uncut.
        let s = slice(&p, 4, 8, 16);
        assert_eq!(s.name, "line_slice_4_8");
        assert_eq!(s.bars, 1);
        assert_eq!(steps(&s), [0, 4]);
        assert_eq!(lens(&s), [8, 8], "tails overrun the slice end");
        // Bars round up: a 20-step window on a 16-step bar is two bars.
        assert_eq!(slice(&p, 0, 20, 16).bars, 2);
        // A window with no attacks is empty but still one bar.
        let empty = slice(&p, 100, 8, 16);
        assert_eq!(empty.bars, 1);
        assert!(empty.notes.is_empty());
    }

    #[test]
    fn transpose_respells_names_and_preserves_the_midi_form() {
        let p = Pattern {
            name: "riff".into(),
            bars: 1,
            notes: vec![note(0, 1, "C4"), note(2, 1, "Gb3"), note(4, 1, "midi:36")],
        };
        let up = transpose(&p, 2).unwrap();
        assert_eq!(up.name, "riff_t+2");
        assert_eq!(names(&up), ["D4", "G#3", "midi:38"]);
        // A kit note stays a kit note — the "midi:" form is preserved.
        assert!(matches!(&up.notes[2].pitch, Value::Note(s) if s == "midi:38"));
        // Transposing by 0 still canonicalizes the spelling (flats → sharps).
        assert_eq!(names(&transpose(&p, 0).unwrap()), ["C4", "F#3", "midi:36"]);
        // Down works, and the suffix records it.
        assert_eq!(transpose(&p, -12).unwrap().name, "riff_t-12");
        assert_eq!(
            names(&transpose(&p, -12).unwrap()),
            ["C3", "F#2", "midi:24"]
        );
    }

    #[test]
    fn transpose_scales_hz_and_leaves_modulators_alone() {
        let slide = Value::Modulated(Modulator::Slide {
            from: 100.0,
            to: 200.0,
            secs: 0.1,
            curve: crate::dsl::Curve::Lin,
        });
        let p = Pattern {
            name: "fx".into(),
            bars: 1,
            notes: vec![
                SeqNote {
                    step: 0,
                    len: 1,
                    pitch: Value::Const(220.0),
                    gain: 1.0,
                },
                SeqNote {
                    step: 4,
                    len: 1,
                    pitch: slide,
                    gain: 1.0,
                },
            ],
        };
        let up = transpose(&p, 12).unwrap();
        assert!(
            matches!(up.notes[0].pitch, Value::Const(hz) if (hz - 440.0).abs() < 1e-4),
            "Hz scales by 2^(12/12) = 2"
        );
        assert!(
            matches!(up.notes[1].pitch, Value::Modulated(_)),
            "a modulated pitch is untouched"
        );
    }

    #[test]
    fn transpose_errors_loudly_instead_of_skipping() {
        // An unparseable name: the music module's error, verbatim.
        let bad = Pattern {
            name: "bad".into(),
            bars: 1,
            notes: vec![note(0, 1, "H4")],
        };
        let err = transpose(&bad, 2).unwrap_err();
        assert!(
            matches!(&err, PatternError::Pitch(MusicError::BadName(m)) if m.contains("\"H4\"")),
            "unexpected: {err}"
        );
        // The DSL's lenient "m69" shorthand is NOT in the strict grammar.
        let lenient = Pattern {
            name: "bad".into(),
            bars: 1,
            notes: vec![note(0, 1, "m69")],
        };
        assert!(matches!(
            transpose(&lenient, 0).unwrap_err(),
            PatternError::Pitch(MusicError::BadName(_))
        ));
        // Past the top of the MIDI range: named, with the fix.
        let high = Pattern {
            name: "high".into(),
            bars: 1,
            notes: vec![note(3, 1, "midi:127")],
        };
        let err = transpose(&high, 1).unwrap_err();
        assert!(
            matches!(&err, PatternError::BadTranspose(m) if m.contains("step 3") && m.contains("midi:127")),
            "unexpected: {err}"
        );
        // Same for a Hz constant pushed out of the finite range.
        let loud = Pattern {
            name: "loud".into(),
            bars: 1,
            notes: vec![SeqNote {
                step: 0,
                len: 1,
                pitch: Value::Const(1e38),
                gain: 1.0,
            }],
        };
        assert!(matches!(
            transpose(&loud, 120).unwrap_err(),
            PatternError::BadTranspose(_)
        ));
    }

    #[test]
    fn stretch_scales_time_exactly() {
        let p = Pattern {
            name: "riff".into(),
            bars: 2,
            notes: vec![note(0, 4, "C2"), note(6, 2, "G2")],
        };
        // Double time: every step and len ×2, bars ×2.
        let wide = stretch(&p, 2, 1, 16).unwrap();
        assert_eq!(wide.name, "riff_stretch_2_1");
        assert_eq!(wide.bars, 4);
        assert_eq!(steps(&wide), [0, 12]);
        assert_eq!(lens(&wide), [8, 4]);
        // Half time of an even pattern: steps/lens/bars all halve.
        let tight = stretch(&p, 1, 2, 16).unwrap();
        assert_eq!(tight.bars, 1);
        assert_eq!(steps(&tight), [0, 3]);
        assert_eq!(lens(&tight), [2, 1]);
        // A hemiola: 3/2 of a pattern built on multiples of 2.
        let hemi = stretch(&p, 3, 2, 16).unwrap();
        assert_eq!(hemi.bars, 3);
        assert_eq!(steps(&hemi), [0, 9]);
        assert_eq!(lens(&hemi), [6, 3]);
    }

    #[test]
    fn stretch_refuses_off_grid_results_naming_the_note() {
        let p = Pattern {
            name: "riff".into(),
            bars: 2,
            notes: vec![note(1, 2, "C2"), note(4, 1, "G2")],
        };
        // Halving lands step 1 between grid points — and says which note.
        let err = stretch(&p, 1, 2, 16).unwrap_err();
        assert_eq!(
            err.to_string(),
            "stretch 1/2 pulls the note at step 1 (pitch \"C2\") off the grid: 1 × 1 isn't \
             divisible by 2 — pick a ratio that keeps every step, len, and the bar count \
             integral, or quantize first"
        );
        assert!(matches!(
            err,
            PatternError::OffGrid {
                value: 1,
                num: 1,
                den: 2,
                ..
            }
        ));
        // The bar count is checked first: a 1-bar pattern can't halve at all.
        let one_bar = Pattern {
            name: "one".into(),
            bars: 1,
            notes: vec![note(0, 2, "C2")],
        };
        let err = stretch(&one_bar, 1, 2, 16).unwrap_err();
        assert!(
            matches!(&err, PatternError::OffGrid { what, value: 1, .. } if what == "the bar count"),
            "unexpected: {err}"
        );
        // A length that won't divide names the note it belongs to.
        let p2 = Pattern {
            name: "riff".into(),
            bars: 2,
            notes: vec![note(2, 1, "G2")],
        };
        let err = stretch(&p2, 1, 2, 16).unwrap_err();
        assert!(
            matches!(&err, PatternError::OffGrid { what, value: 1, .. } if what.contains("step 2")),
            "unexpected: {err}"
        );
        // A degenerate ratio is BadParams, not a division by zero.
        assert!(matches!(
            stretch(&p, 0, 2, 16).unwrap_err(),
            PatternError::BadParams(_)
        ));
        assert!(matches!(
            stretch(&p, 1, 0, 16).unwrap_err(),
            PatternError::BadParams(_)
        ));
    }

    #[test]
    fn rotate_wraps_around_the_pattern_length() {
        let p = Pattern {
            name: "riff".into(),
            bars: 1,
            notes: vec![note(0, 2, "C2"), note(4, 2, "E2"), note(12, 2, "G2")],
        };
        // +4: the last note wraps 12 → 0 and leads.
        let r = rotate(&p, 4, 16);
        assert_eq!(r.name, "riff_rot+4");
        assert_eq!(r.bars, 1);
        assert_eq!(steps(&r), [0, 4, 8]);
        assert_eq!(names(&r), ["G2", "C2", "E2"]);
        // Negative shifts wrap the other way.
        let back = rotate(&p, -4, 16);
        assert_eq!(steps(&back), [0, 8, 12]);
        assert_eq!(names(&back), ["E2", "G2", "C2"]);
        // A full turn is the identity; 20 ≡ 4 (mod 16).
        assert_eq!(steps(&rotate(&p, 16, 16)), [0, 4, 12]);
        assert_eq!(steps(&rotate(&p, 20, 16)), steps(&rotate(&p, 4, 16)));
        // Rotating twice by complementary shifts returns the original.
        let round = rotate(&rotate(&p, 7, 16), -7, 16);
        assert_eq!(steps(&round), [0, 4, 12]);
        assert_eq!(names(&round), ["C2", "E2", "G2"]);
    }

    #[test]
    fn reverse_mirrors_note_intervals() {
        let p = Pattern {
            name: "riff".into(),
            bars: 1,
            notes: vec![note(0, 4, "C2"), note(8, 2, "G2")],
        };
        // [0,4) → [12,16); [8,10) → [6,8).
        let r = reverse(&p, 16);
        assert_eq!(r.name, "riff_rev");
        assert_eq!(r.bars, 1);
        assert_eq!(steps(&r), [6, 12]);
        assert_eq!(lens(&r), [2, 4], "lengths mirror with their notes");
        assert_eq!(names(&r), ["G2", "C2"]);
        // Mirroring twice returns the original.
        let twice = reverse(&reverse(&p, 16), 16);
        assert_eq!(steps(&twice), [0, 8]);
        assert_eq!(lens(&twice), [4, 2]);
    }

    #[test]
    fn quantize_snaps_starts_halves_away_from_zero() {
        let p = Pattern {
            name: "loose".into(),
            bars: 1,
            notes: vec![
                note(1, 3, "C4"),
                note(2, 3, "D4"),
                note(3, 3, "E4"),
                note(5, 3, "F4"),
                note(6, 3, "G4"),
                note(7, 3, "A4"),
            ],
        };
        let q = quantize(&p, 4);
        assert_eq!(q.name, "loose_q4");
        // 1→0, 2→4 (the half rounds AWAY from zero), 3→4, 5→4, 6→8, 7→8.
        assert_eq!(steps(&q), [0, 4, 4, 4, 8, 8]);
        assert_eq!(lens(&q), [3; 6], "lengths are untouched");
        // Grid 1 is the identity.
        assert_eq!(steps(&quantize(&p, 1)), [1, 2, 3, 5, 6, 7]);
    }

    #[test]
    fn vel_scales_and_clamps_gains() {
        let p = Pattern {
            name: "riff".into(),
            bars: 1,
            notes: vec![note_vel(0, 1, "C2", 1.0), note_vel(4, 1, "G2", 0.5)],
        };
        let up = vel(&p, 1.5);
        assert_eq!(up.name, "riff_vel1.5");
        assert_eq!(up.notes[0].gain, 1.0, "clamped at the 0..1 convention");
        assert_eq!(up.notes[1].gain, 0.75);
        let down = vel(&p, 0.5);
        assert_eq!(down.notes[0].gain, 0.5);
        assert_eq!(down.notes[1].gain, 0.25);
    }

    #[test]
    fn gate_shortens_lengths_with_a_floor_of_one() {
        let p = Pattern {
            name: "riff".into(),
            bars: 1,
            notes: vec![note(0, 4, "C2"), note(4, 2, "E2"), note(8, 1, "G2")],
        };
        let staccato = gate(&p, 0.5);
        assert_eq!(staccato.name, "riff_gate0.5");
        assert_eq!(lens(&staccato), [2, 1, 1], "0.5 = staccato, floored at 1");
        assert_eq!(lens(&gate(&p, 2.0)), [8, 4, 2]);
        // A degenerate factor still can't kill a note.
        assert_eq!(lens(&gate(&p, 0.0)), [1, 1, 1]);
        assert_eq!(steps(&staccato), [0, 4, 8], "starts are untouched");
    }

    #[test]
    fn probability_is_deterministic_per_note_identity() {
        // 32 distinct notes; keep half of them.
        let p = Pattern {
            name: "line".into(),
            bars: 2,
            notes: (0..32).map(|i| note(i, 1, "C4")).collect(),
        };
        let a = probability(&p, 0.5, 7);
        let b = probability(&p, 0.5, 7);
        assert_eq!(
            steps(&a),
            steps(&b),
            "same pattern + same seed ⇒ same drops"
        );
        assert_eq!(a.name, "line_prob0.5");
        let c = probability(&p, 0.5, 8);
        assert_ne!(steps(&a), steps(&c), "a different seed redraws");
        // The draw follows the NOTE, not the vec position: shuffle the input
        // and the same notes survive.
        let mut shuffled = p.clone();
        shuffled.notes.reverse();
        let mut kept_a = steps(&a);
        kept_a.sort_unstable();
        let mut kept_shuffled = steps(&probability(&shuffled, 0.5, 7));
        kept_shuffled.sort_unstable();
        assert_eq!(kept_a, kept_shuffled);
        // The extremes: keep everything / drop everything.
        assert_eq!(probability(&p, 1.0, 7).notes.len(), 32);
        assert!(probability(&p, 0.0, 7).notes.is_empty());
        // Duplicate notes (same step, len, pitch) share one draw.
        let dupes = Pattern {
            name: "d".into(),
            bars: 1,
            notes: vec![note(4, 2, "C4"), note(4, 2, "C4")],
        };
        let kept = probability(&dupes, 0.5, 7).notes.len();
        assert!(kept == 0 || kept == 2, "duplicates drop or keep together");
    }

    #[test]
    fn euclidean_spreads_pulses_bresenham_evenly() {
        let clave = euclidean("clave", 3, 8, "midi:36", 2, 16).unwrap();
        assert_eq!(clave.name, "clave");
        assert_eq!(clave.bars, 1);
        assert_eq!(steps(&clave), [0, 3, 6], "the tresillo");
        assert_eq!(lens(&clave), [2, 2, 2]);
        assert_eq!(names(&clave), ["midi:36"; 3]);
        assert!(clave.notes.iter().all(|n| n.gain == 1.0));
        // The other classic: E(5,8).
        let cinq = euclidean("cinq", 5, 8, "midi:42", 1, 16).unwrap();
        assert_eq!(steps(&cinq), [0, 2, 4, 5, 7]);
        // Full density hits every step; zero pulses is an empty pattern.
        assert_eq!(
            steps(&euclidean("full", 8, 8, "midi:36", 1, 16).unwrap()),
            [0, 1, 2, 3, 4, 5, 6, 7]
        );
        assert!(
            euclidean("none", 0, 8, "midi:36", 1, 16)
                .unwrap()
                .notes
                .is_empty()
        );
        // A cycle longer than a bar spans bars.
        assert_eq!(euclidean("long", 3, 32, "midi:36", 1, 16).unwrap().bars, 2);
        // More pulses than positions is an error, not a truncation.
        let err = euclidean("dense", 9, 8, "midi:36", 1, 16).unwrap_err();
        assert!(
            matches!(&err, PatternError::BadParams(m) if m.contains("9") && m.contains("8")),
            "unexpected: {err}"
        );
        assert!(matches!(
            euclidean("zero", 1, 0, "midi:36", 1, 16).unwrap_err(),
            PatternError::BadParams(_)
        ));
    }

    #[test]
    fn tuplet_spaces_notes_evenly() {
        // Three in the time of eight steps: 0, 3, 5 (rounds of 0, 2.67, 5.33).
        let triplet = tuplet("trip", 3, 8, "C5", 1);
        assert_eq!(triplet.name, "trip");
        assert_eq!(triplet.bars, 1);
        assert_eq!(steps(&triplet), [0, 3, 5]);
        assert_eq!(lens(&triplet), [1; 3]);
        assert_eq!(names(&triplet), ["C5"; 3]);
        // Two across three steps: 1.5 rounds AWAY from zero → 0, 2.
        assert_eq!(steps(&tuplet("du", 2, 3, "C5", 1)), [0, 2]);
        // Four across sixteen lands on the quarter grid.
        assert_eq!(steps(&tuplet("q", 4, 16, "C5", 1)), [0, 4, 8, 12]);
        // No count, no notes — still a one-bar pattern.
        assert!(tuplet("none", 0, 8, "C5", 1).notes.is_empty());
    }

    #[test]
    fn humanize_is_deterministic_bounded_and_identity_based() {
        let p = Pattern {
            name: "line".into(),
            bars: 2,
            notes: (0..32).map(|i| note_vel(i, 1, "C4", 0.8)).collect(),
        };
        // Zero amounts are an exact no-op on the notes (name aside).
        let noop = humanize(&p, 0.0, 0.0, 42);
        assert_eq!(noop.name, "line_hum");
        assert_eq!(steps(&noop), steps(&p));
        assert_eq!(lens(&noop), lens(&p));
        // Same seed ⇒ same jitter; different seed ⇒ different jitter.
        let a = humanize(&p, 1.0, 0.5, 42);
        assert_eq!(steps(&a), steps(&humanize(&p, 1.0, 0.5, 42)));
        assert_ne!(
            steps(&a),
            steps(&humanize(&p, 1.0, 0.5, 43)),
            "a different seed redraws"
        );
        // Jitter stays inside its bounds: steps ≥ 0, gains within 0..1.
        for seed in 0..8 {
            let h = humanize(&p, 2.0, 1.0, seed);
            for n in &h.notes {
                assert!((0.0..=1.0).contains(&n.gain));
                assert!(n.step < 34, "jitter is bounded by ±timing");
            }
        }
        // The draw follows the note identity: same notes, shuffled order,
        // same per-note jitter (compared as a pitch-independent step multiset
        // would conflate duplicates, so key by the note's original position —
        // here the steps are unique, making the multiset exact).
        let mut shuffled = p.clone();
        shuffled.notes.reverse();
        let mut ja = steps(&humanize(&p, 1.0, 0.0, 9));
        ja.sort_unstable();
        let mut jb = steps(&humanize(&shuffled, 1.0, 0.0, 9));
        jb.sort_unstable();
        assert_eq!(ja, jb);
    }

    #[test]
    fn transform_names_read_as_derived() {
        let p = riff();
        assert_eq!(repeat(&p, 16, 2).name, "riff_x2");
        assert_eq!(reverse(&p, 16).name, "riff_rev");
        assert_eq!(transpose(&p, 5).unwrap().name, "riff_t+5");
        assert_eq!(rotate(&p, 3, 16).name, "riff_rot+3");
        assert_eq!(rotate(&p, -2, 16).name, "riff_rot-2");
        assert_eq!(quantize(&p, 4).name, "riff_q4");
        assert_eq!(vel(&p, 0.9).name, "riff_vel0.9");
        assert_eq!(gate(&p, 0.75).name, "riff_gate0.75");
        assert_eq!(probability(&p, 0.5, 1).name, "riff_prob0.5");
        assert_eq!(humanize(&p, 0.5, 0.1, 1).name, "riff_hum");
        assert_eq!(slice(&p, 4, 8, 16).name, "riff_slice_4_8");
        assert_eq!(stretch(&p, 2, 1, 16).unwrap().name, "riff_stretch_2_1");
    }

    #[test]
    fn transforms_compose_into_a_pattern_that_still_compiles() {
        let mut p = Pattern {
            name: "riff".into(),
            bars: 1,
            notes: vec![
                note(0, 4, "C3"),
                note_vel(4, 2, "E3", 0.9),
                note(8, 4, "G3"),
                note_vel(12, 2, "B3", 0.7),
            ],
        };
        p = repeat(&p, 16, 2);
        p = transpose(&p, 5).unwrap();
        p = reverse(&p, 16);
        p = rotate(&p, 3, 16);
        p = quantize(&p, 1);
        p = gate(&p, 0.75);
        p = vel(&p, 0.9);
        p = humanize(&p, 0.0, 0.0, 42); // zero amounts: exact no-op
        p = probability(&p, 1.0, 99); // keep everything
        let drums = euclidean("kick", 4, 32, "midi:36", 1, 16).unwrap();
        p = layer(&p, &drums);
        p = concat(&p, &tuplet("trip", 3, 8, "C5", 1), 16);
        p = slice(&p, 0, 40, 16);
        p = stretch(&p, 1, 1, 16).unwrap();

        // The composed result is musically sane: notes exist inside the grid,
        // lengths and velocities stay in convention, and every pitch name
        // still parses strictly.
        assert_eq!(p.bars, 3);
        assert!(!p.notes.is_empty());
        let total = p.bars as u64 * 16;
        for n in &p.notes {
            assert!(n.len >= 1);
            assert!((0.0..=1.0).contains(&n.gain));
            assert!((n.step as u64) < total);
            if let Value::Note(name) = &n.pitch {
                Pitch::from_name(name)
                    .unwrap_or_else(|e| panic!("{name} must stay parseable: {e}"));
            }
        }
        // And it drops back into a song and compiles like any authored phrase.
        let mut song = Song::new("roundtrip", 120.0);
        song.add_track("keys", SeqWave::Epiano, Adsr::new(0.005, 0.2, 0.6, 0.2));
        song.add_pattern(p.name.clone(), p.bars, p.notes.clone());
        song.arrange("keys", p.name.clone(), 0);
        song.to_doc().unwrap();
    }
}
