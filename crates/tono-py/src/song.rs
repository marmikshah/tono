//! The typed song API: `tono.Song` / `tono.Pattern` / `tono.Track` /
//! `tono.Program` wrap the native Rust objects directly — building, compiling,
//! and rendering a song never crosses a JSON boundary. Rust owns semantics, so
//! an equivalent song compiles to the same Program hash from either language
//! (`crates/tono-core/tests/equivalence.rs` pins the contract).
//!

use std::collections::HashMap;
use std::sync::Arc;

use numpy::{IntoPyArray, PyArray1, PyArray2, PyArrayMethods};
use pyo3::exceptions::{PyOSError, PyTypeError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::{PyDict, PyList, PyType};

use tono_core::catalog::{self, Voice as CoreVoice};
use tono_core::diag::{CompileError as CoreCompileError, Diagnostic};
use tono_core::dsl::{AutoCurve, AutoTarget, Bus, Node, Send, SeqWave, TempoPoint};
use tono_core::program::Program as CoreProgram;
use tono_core::song::{
    CompileOptions, CompileTarget, Marker, MeterPoint, Pattern as CorePattern, Phrase, Section,
    Song as CoreSong, SongLane, SongPoint, SongTrack,
};
use tono_core::units::Beat;

/// The typed API's grid: 4 steps per beat (sixteenth notes) — the same
/// `steps_per_beat` `Song::new` stamps, shared by the pattern write cursor and
/// every pattern op below.
const STEPS_PER_BEAT: u32 = 4;
/// The default meter's beats per bar (`Song::new`'s `beats_per_bar`).
const BEATS_PER_BAR: u32 = 4;
/// Steps per bar on that grid — the `steps_per_bar` argument every pattern op
/// (`repeat`, `concat`, `slice`, `rotate`, `reverse`, `euclidean`) runs on.
const STEPS_PER_BAR: u32 = STEPS_PER_BEAT * BEATS_PER_BAR;

pyo3::create_exception!(
    tono,
    TonoError,
    pyo3::exceptions::PyException,
    "The base error for every typed-API failure (program load, …)."
);
pyo3::create_exception!(
    tono,
    CompileError,
    TonoError,
    "A song failed to compile. `.diagnostics` is a list of dicts — \
     {code, severity, path, message, remediation?} — one per problem found."
);

/// The document string of a seq wave (`SeqWave` serializes lowercase).
fn wave_str(wave: SeqWave) -> String {
    serde_json::to_value(wave)
        .expect("a SeqWave serializes")
        .as_str()
        .expect("a SeqWave serializes to a string")
        .to_owned()
}

/// Diagnostics as Python dicts: {code, severity, path, message, remediation?}.
fn diagnostics_list<'py>(
    py: Python<'py>,
    diagnostics: &[Diagnostic],
) -> PyResult<Bound<'py, PyList>> {
    let list = PyList::empty(py);
    for d in diagnostics {
        let dict = PyDict::new(py);
        dict.set_item("code", d.code)?;
        dict.set_item("severity", d.severity.to_string())?;
        dict.set_item("path", &d.path)?;
        dict.set_item("message", &d.message)?;
        if let Some(remediation) = &d.remediation {
            dict.set_item("remediation", remediation)?;
        }
        list.append(dict)?;
    }
    Ok(list)
}

/// Map a core compile failure to the Python `CompileError`, with the
/// structured diagnostics attached as `.diagnostics`.
fn compile_error(py: Python<'_>, err: CoreCompileError) -> PyErr {
    let pyerr = CompileError::new_err(err.to_string());
    match diagnostics_list(py, &err.0) {
        Ok(list) => {
            // Attaching to a fresh exception instance cannot realistically
            // fail; worst case the exception carries only its message.
            let _ = pyerr.value(py).setattr("diagnostics", list);
        }
        Err(e) => return e,
    }
    pyerr
}

/// A `(num, den)` pair to a normalized beat, validating the denominator.
fn beat_from_parts(num: i64, den: i64) -> PyResult<Beat> {
    if den <= 0 || den > i64::from(u32::MAX) {
        return Err(PyValueError::new_err(format!(
            "beat denominator must be in 1..={}, got {den}",
            u32::MAX
        )));
    }
    Ok(Beat::new(num, den as u32))
}

/// A float to a beat by its EXACT binary value (mantissa × 2^exponent): 0.5
/// is exactly 1/2, and float 0.1 is 0.1's binary expansion — NOT 1/10 (pass
/// `fractions.Fraction` for exact decimals). Values whose exact form doesn't
/// fit the beat rational (i64/u32) are rejected, pointing at `Fraction`.
fn beat_from_f64(x: f64) -> PyResult<Beat> {
    if !x.is_finite() {
        return Err(PyValueError::new_err(format!(
            "a beat must be finite, got {x}"
        )));
    }
    if x == 0.0 {
        return Ok(Beat::zero());
    }
    // IEEE-754 double: value = ±mantissa × 2^e2.
    let bits = x.to_bits();
    let raw_exp = ((bits >> 52) & 0x7ff) as i32;
    let frac = bits & ((1u64 << 52) - 1);
    let (mantissa, e2) = if raw_exp == 0 {
        (frac, -1074i32) // subnormal: frac × 2^-1074
    } else {
        (frac | (1u64 << 52), raw_exp - 1075) // normal: (2^52 | frac) × 2^(exp-1075)
    };
    let mut m = mantissa as i64;
    if bits >> 63 == 1 {
        m = -m;
    }
    // Strip trailing zeros so the denominator stays as small as possible.
    let tz = m.trailing_zeros();
    m >>= tz;
    let e2 = e2 + tz as i32;
    let inexact = |what: &str| {
        PyValueError::new_err(format!(
            "float {x} {what} — pass fractions.Fraction for exact decimals \
             (e.g. Fraction(1, 10) for 1/10 of a beat)"
        ))
    };
    if e2 >= 0 {
        if e2 > 62 {
            return Err(inexact("is too large to represent exactly as a beat"));
        }
        let num = i64::try_from((m as i128) << (e2 as u32))
            .map_err(|_| inexact("is too large to represent exactly as a beat"))?;
        Ok(Beat::from_int(num))
    } else {
        let shift = (-e2) as u32;
        if shift >= 32 {
            return Err(inexact("has no exact beat representation"));
        }
        Ok(Beat::new(m, 1u32 << shift))
    }
}

/// Convert a Python beat position to an exact `units::Beat` — the one
/// conversion every beat-taking method (`set_tempo_map`, `set_pickup`,
/// `add_marker`) shares. Accepts an int (whole beats), a float (its EXACT
/// binary value — see [`beat_from_f64`]), a `fractions.Fraction` (exact
/// decimals), or a `(num, den)` int tuple. Anything else is a TypeError
/// naming the accepted forms.
fn py_beat(obj: &Bound<'_, PyAny>) -> PyResult<Beat> {
    if let Ok(whole) = obj.extract::<i64>() {
        return Ok(Beat::from_int(whole));
    }
    // fractions.Fraction (duck-typed: integer numerator/denominator) — before
    // the float arm, since a Fraction's __float__ would round it to a double.
    if let (Ok(num), Ok(den)) = (
        obj.getattr("numerator").and_then(|n| n.extract::<i64>()),
        obj.getattr("denominator").and_then(|d| d.extract::<i64>()),
    ) {
        return beat_from_parts(num, den);
    }
    if let Ok((num, den)) = obj.extract::<(i64, i64)>() {
        return beat_from_parts(num, den);
    }
    if let Ok(x) = obj.extract::<f64>() {
        return beat_from_f64(x);
    }
    let type_name = obj
        .get_type()
        .name()
        .map(|n| n.to_string())
        .unwrap_or_else(|_| "?".into());
    Err(PyTypeError::new_err(format!(
        "a beat must be an int (whole beats), a float, a fractions.Fraction, or a \
         (num, den) int tuple — got {type_name:?}"
    )))
}

/// The node types valid in a bus insert chain — `dsl::Node::is_processor`'s
/// set by serde tag, so `add_bus` can reject a non-processor (or unknown)
/// effect type before serde, with the full accepted list in the message.
const PROCESSOR_TYPES: &[&str] = &[
    "lowpass",
    "highpass",
    "bandpass",
    "notch",
    "peak",
    "lowshelf",
    "highshelf",
    "gain",
    "bitcrush",
    "downsample",
    "delay",
    "reverb",
    "modal",
    "drive",
    "ringmod",
    "tremolo",
    "chorus",
    "flanger",
    "phaser",
    "compress",
    "duck",
    "convolve",
    "granular",
];

/// A Python value to JSON for effect params: None/bool/int/float/str, and
/// dicts/lists recursively (a modulator param like `{"type": "lfo", ...}` is
/// just a nested dict).
fn py_to_json(obj: &Bound<'_, PyAny>) -> PyResult<serde_json::Value> {
    if obj.is_none() {
        return Ok(serde_json::Value::Null);
    }
    if let Ok(b) = obj.extract::<bool>() {
        return Ok(b.into());
    }
    if let Ok(i) = obj.extract::<i64>() {
        return Ok(i.into());
    }
    if let Ok(f) = obj.extract::<f64>() {
        return serde_json::Number::from_f64(f)
            .map(serde_json::Value::Number)
            .ok_or_else(|| {
                PyValueError::new_err(format!("non-finite number {f} in effect params"))
            });
    }
    if let Ok(s) = obj.extract::<String>() {
        return Ok(s.into());
    }
    if let Ok(dict) = obj.cast::<PyDict>() {
        let mut map = serde_json::Map::new();
        for (k, v) in dict.iter() {
            map.insert(k.extract::<String>()?, py_to_json(&v)?);
        }
        return Ok(serde_json::Value::Object(map));
    }
    if let Ok(items) = obj.extract::<Vec<Bound<'_, PyAny>>>() {
        return Ok(serde_json::Value::Array(
            items.iter().map(py_to_json).collect::<PyResult<Vec<_>>>()?,
        ));
    }
    let type_name = obj
        .get_type()
        .name()
        .map(|n| n.to_string())
        .unwrap_or_else(|_| "?".into());
    Err(PyTypeError::new_err(format!(
        "unsupported effect param value of type {type_name:?} — use None, bool, int, float, \
         str, list, or dict"
    )))
}

/// Build one bus effect node from a `(type, params)` pair, e.g.
/// `("reverb", {"room": 0.5, "mix": 0.3})`. Unknown or non-processor types
/// are ValueErrors naming the accepted processor types; a known type with bad
/// or unknown params is a ValueError naming the node and its fields.
fn build_effect(kind: &str, params: &Bound<'_, PyAny>) -> PyResult<Node> {
    if !PROCESSOR_TYPES.contains(&kind) {
        return Err(PyValueError::new_err(format!(
            "unknown effect type {kind:?} — bus effects must be processor nodes, one of: {}",
            PROCESSOR_TYPES.join(", ")
        )));
    }
    let dict = params.cast::<PyDict>().map_err(|_| {
        PyTypeError::new_err(format!(
            "effect {kind:?} params must be a dict, e.g. ({kind:?}, {{...}})"
        ))
    })?;
    let mut value = serde_json::Map::new();
    value.insert("type".into(), kind.into());
    for (k, v) in dict.iter() {
        value.insert(k.extract::<String>()?, py_to_json(&v)?);
    }
    let node: Node = serde_json::from_value(serde_json::Value::Object(value))
        .map_err(|e| PyValueError::new_err(format!("effect {kind:?}: {e}")))?;
    // serde fills missing fields with defaults but would also silently drop an
    // unknown param — catch that by round-tripping: every given key must
    // survive the trip through the typed node.
    let round_trip = serde_json::to_value(&node).expect("a node serializes");
    let fields = round_trip
        .as_object()
        .expect("a node serializes to an object");
    let unknown: Vec<String> = dict
        .iter()
        .filter_map(|(k, _)| k.extract::<String>().ok())
        .filter(|k| !fields.contains_key(k))
        .collect();
    if !unknown.is_empty() {
        let accepted: Vec<&str> = fields.keys().map(String::as_str).collect();
        return Err(PyValueError::new_err(format!(
            "effect {kind:?}: unknown param(s) {} — accepted: {}",
            unknown.join(", "),
            accepted.join(", ")
        )));
    }
    Ok(node)
}

/// Resolve a `Track` handle or a track name string to the name (shared by
/// `arrange` and `automate`).
fn track_name_of(track: &Bound<'_, PyAny>) -> PyResult<String> {
    if let Ok(cell) = track.cast::<Track>() {
        Ok(cell.borrow().name.clone())
    } else if let Ok(name) = track.extract::<String>() {
        Ok(name)
    } else {
        Err(PyValueError::new_err(
            "track must be a Track or a track name string",
        ))
    }
}

/// A catalog instrument voice — what a `Song` track plays. Construct one with
/// the `tono.instruments` functions; tune it with the builder methods.
#[pyclass(module = "tono")]
struct Voice {
    inner: CoreVoice,
}

#[pymethods]
impl Voice {
    /// The voice's display name (read-only).
    #[getter]
    fn name(&self) -> String {
        self.inner.name.clone()
    }

    /// The synthesis wave the voice plays, as its document string (`"piano"`,
    /// `"bass"`, `"kit"`, …) (read-only).
    #[getter]
    fn wave(&self) -> String {
        wave_str(self.inner.wave)
    }

    /// Set the channel fader (0..2, 1 = unity); returns the voice for chaining.
    fn gain<'py>(mut slf: PyRefMut<'py, Self>, x: f32) -> PyRefMut<'py, Self> {
        slf.inner.gain = x;
        slf
    }

    /// Set the stereo position (−1 hard left .. 1 hard right); returns the
    /// voice for chaining.
    fn pan<'py>(mut slf: PyRefMut<'py, Self>, x: f32) -> PyRefMut<'py, Self> {
        slf.inner.pan = x;
        slf
    }

    /// Set the reverb send (0..1, 0 = dry); returns the voice for chaining.
    fn reverb<'py>(mut slf: PyRefMut<'py, Self>, x: f32) -> PyRefMut<'py, Self> {
        slf.inner.reverb = x;
        slf
    }

    /// Override the song's swing for this track (0..1); returns the voice for
    /// chaining.
    fn swing<'py>(mut slf: PyRefMut<'py, Self>, x: f32) -> PyRefMut<'py, Self> {
        slf.inner.swing = Some(x);
        slf
    }

    /// Override the song's humanize for this track (0..1); returns the voice
    /// for chaining.
    fn humanize<'py>(mut slf: PyRefMut<'py, Self>, x: f32) -> PyRefMut<'py, Self> {
        slf.inner.humanize = Some(x);
        slf
    }

    fn __repr__(&self) -> String {
        format!("Voice('{}')", self.inner.name)
    }
}

/// Reject an unknown instrument variant, naming the valid ones.
fn unknown_variant(family: &str, variant: &str, valid: &[&str]) -> PyErr {
    PyValueError::new_err(format!(
        "unknown {family} variant {variant:?} — expected one of: {}",
        valid.join(", ")
    ))
}

/// One `tono.instruments` family function: variant slug → catalog constructor
/// (the same slugs the `tono catalog` CLI lists).
macro_rules! instrument_fn {
    ($name:ident, $doc:literal, $family:literal, $default:literal, $($slug:literal => $ctor:path),+ $(,)?) => {
        #[doc = $doc]
        #[pyfunction]
        #[pyo3(signature = (variant = $default))]
        fn $name(variant: &str) -> PyResult<Voice> {
            let inner = match variant {
                $($slug => $ctor(),)+
                other => return Err(unknown_variant($family, other, &[$($slug),+])),
            };
            Ok(Voice { inner })
        }
    };
}

instrument_fn!(
    piano,
    "A piano voice — variants: grand (default), bright, mellow, felt, upright, honky-tonk.",
    "piano",
    "grand",
    "grand" => catalog::GrandPiano::grand,
    "bright" => catalog::GrandPiano::bright,
    "mellow" => catalog::GrandPiano::mellow,
    "felt" => catalog::GrandPiano::felt,
    "upright" => catalog::GrandPiano::upright,
    "honky-tonk" => catalog::GrandPiano::honky_tonk,
);
instrument_fn!(
    electric_piano,
    "An electric piano voice — variants: rhodes (default), wurli, dx.",
    "electric_piano",
    "rhodes",
    "rhodes" => catalog::ElectricPiano::rhodes,
    "wurli" => catalog::ElectricPiano::wurli,
    "dx" => catalog::ElectricPiano::dx,
);
instrument_fn!(
    organ,
    "An organ voice — variants: tonewheel (default), rock.",
    "organ",
    "tonewheel",
    "tonewheel" => catalog::Organ::tonewheel,
    "rock" => catalog::Organ::rock,
);
instrument_fn!(
    strings,
    "A string ensemble voice — variants: warm (default), ensemble.",
    "strings",
    "warm",
    "warm" => catalog::Strings::warm,
    "ensemble" => catalog::Strings::ensemble,
);
instrument_fn!(
    bass,
    "A bass voice — variants: finger (default), pick, sub, synth.",
    "bass",
    "finger",
    "finger" => catalog::Bass::finger,
    "pick" => catalog::Bass::pick,
    "sub" => catalog::Bass::sub,
    "synth" => catalog::Bass::synth,
);
instrument_fn!(
    guitar,
    "A guitar voice — variants: nylon (default), steel, electric.",
    "guitar",
    "nylon",
    "nylon" => catalog::Guitar::nylon,
    "steel" => catalog::Guitar::steel,
    "electric" => catalog::Guitar::electric,
);
instrument_fn!(
    drums,
    "A General MIDI drum kit — variants: acoustic (default), classic, electronic, tr808.",
    "drums",
    "acoustic",
    "acoustic" => catalog::Drums::acoustic,
    "classic" => catalog::Drums::classic,
    "electronic" => catalog::Drums::electronic,
    "tr808" => catalog::Drums::tr808,
);
instrument_fn!(
    brass,
    "A brass voice — variants: section (default), stab.",
    "brass",
    "section",
    "section" => catalog::Brass::section,
    "stab" => catalog::Brass::stab,
);
instrument_fn!(
    flute,
    "A flute voice — variants: concert (default).",
    "flute",
    "concert",
    "concert" => catalog::Flute::concert,
);
instrument_fn!(
    mallets,
    "A mallets voice — variants: marimba (default), vibraphone, glockenspiel.",
    "mallets",
    "marimba",
    "marimba" => catalog::Mallets::marimba,
    "vibraphone" => catalog::Mallets::vibraphone,
    "glockenspiel" => catalog::Mallets::glockenspiel,
);
instrument_fn!(
    bells,
    "A bells voice — variants: tubular (default).",
    "bells",
    "tubular",
    "tubular" => catalog::Bells::tubular,
);

/// A reusable musical phrase, `bars` long, on the typed API's grid of 4 steps
/// per beat / 16 steps per bar (songs created through this API keep that
/// default grid). Notes are placed by beat (floats welcome: beat 0.5 is the
/// second eighth note) and snap to the grid exactly like the Rust `Phrase`
/// writer. The transform methods (`transpose`, `stretch`, `reverse`, …) are
/// pure: each returns a NEW pattern and never mutates this one.
#[pyclass(module = "tono")]
struct Pattern {
    /// The core pattern (name/bars/notes): the value the ops consume and the
    /// arrangement registers. Owns the pattern's notes.
    inner: CorePattern,
    /// The write cursor: `note`/`notes`/`hit`/`chord` place through it (its
    /// snapping IS the Rust `Phrase` semantics the equivalence hash pins).
    /// Written notes are immediately moved into `inner`.
    phrase: Phrase,
}

impl Pattern {
    /// Wrap a finished core pattern (an op's output): notes fixed, a fresh
    /// write cursor so later writes append after them.
    fn from_core(inner: CorePattern) -> Self {
        Pattern {
            inner,
            phrase: Phrase::new(STEPS_PER_BEAT),
        }
    }
}

#[pymethods]
impl Pattern {
    /// An empty pattern `bars` long.
    #[new]
    #[pyo3(signature = (bars=1))]
    fn new(bars: u32) -> Self {
        Pattern {
            inner: CorePattern {
                name: "pattern".into(),
                bars: bars.max(1),
                notes: Vec::new(),
            },
            phrase: Phrase::new(STEPS_PER_BEAT),
        }
    }

    /// `pulses` hits Bresenham-evenly across `steps` grid positions — the
    /// euclidean-rhythm construction (`euclidean(3, 8, "midi:36")` is the
    /// tresillo, hits at 0, 3, 6). Every hit gets `pitch` and length `len`
    /// steps at full velocity. `bars` lengthens the pattern the cycle sits in
    /// (the result is at least `ceil(steps / 16)` bars — the grid is 16 steps
    /// per bar). More pulses than steps is a ValueError.
    #[classmethod]
    #[pyo3(signature = (pulses, steps, pitch, len=1, bars=1))]
    fn euclidean(
        _cls: &Bound<'_, PyType>,
        pulses: u32,
        steps: u32,
        pitch: &str,
        len: u32,
        bars: u32,
    ) -> PyResult<Self> {
        let mut inner =
            tono_core::song::euclidean("euclidean", pulses, steps, pitch, len, STEPS_PER_BAR)
                .map_err(|e| PyValueError::new_err(e.to_string()))?;
        inner.bars = inner.bars.max(bars.max(1));
        Ok(Pattern::from_core(inner))
    }

    /// `count` notes spaced evenly across `in_steps` steps — the
    /// triplet/quintuplet constructor: position `round(i × in_steps / count)`
    /// (halves away from zero), each `len` steps long at full velocity, in a
    /// one-bar pattern (`repeat` or `stretch` it to span more).
    #[classmethod]
    #[pyo3(signature = (count, in_steps, pitch, len=1))]
    fn tuplet(_cls: &Bound<'_, PyType>, count: u32, in_steps: u32, pitch: &str, len: u32) -> Self {
        Pattern::from_core(tono_core::song::tuplet(
            "tuplet", count, in_steps, pitch, len,
        ))
    }

    /// The pattern's length in bars.
    #[getter]
    fn bars(&self) -> u32 {
        self.inner.bars
    }

    /// Place a note at beat `at`, `duration` beats long, with velocity `gain`
    /// (0..1). `pitch` is a note name (`"C4"`, `"F#3"`), a MIDI note
    /// (`"midi:36"`), or Hz.
    #[pyo3(signature = (pitch, at=0.0, duration=1.0, gain=1.0))]
    fn note(&mut self, pitch: &str, at: f32, duration: f32, gain: f32) {
        self.phrase.at(at).vel(gain).note(pitch, duration);
        self.inner.notes.extend(self.phrase.take_notes());
    }

    /// Place `pitches` one after another from the pattern's start (the cursor
    /// advances past each note). `durations` is one float (the same length for
    /// every note) or a list of floats (one per pitch).
    #[pyo3(signature = (pitches, durations=None))]
    fn notes(
        &mut self,
        pitches: Vec<String>,
        durations: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<()> {
        let durs: Vec<f32> = match durations {
            None => vec![1.0; pitches.len()],
            Some(obj) => {
                if let Ok(d) = obj.extract::<f32>() {
                    vec![d; pitches.len()]
                } else if let Ok(ds) = obj.extract::<Vec<f32>>() {
                    if ds.len() != pitches.len() {
                        return Err(PyValueError::new_err(format!(
                            "durations has {} entries but {} pitches were given — \
                             pass one float or one per pitch",
                            ds.len(),
                            pitches.len()
                        )));
                    }
                    ds
                } else {
                    return Err(PyValueError::new_err(
                        "durations must be a float or a list of floats",
                    ));
                }
            }
        };
        for (pitch, dur) in pitches.iter().zip(durs) {
            self.phrase.play(pitch, dur);
        }
        self.inner.notes.extend(self.phrase.take_notes());
        Ok(())
    }

    /// Hit a drum at each beat in `beats` — a one-step GM hit, meaningful on a
    /// drums voice. `drum` is one of: kick, snare, hat, openhat, clap, crash,
    /// ride, tom.
    fn hit(&mut self, drum: &str, beats: Vec<f32>) -> PyResult<()> {
        // Mirrors the GM constants of Phrase's drum helpers.
        let gm = match drum {
            "kick" => 36,
            "snare" => 38,
            "hat" => 42,
            "openhat" => 46,
            "clap" => 39,
            "crash" => 49,
            "ride" => 51,
            "tom" => 45,
            other => {
                return Err(PyValueError::new_err(format!(
                    "unknown drum {other:?} — expected one of: \
                     kick, snare, hat, openhat, clap, crash, ride, tom"
                )));
            }
        };
        for b in beats {
            self.phrase.at(b).hit(gm);
        }
        self.inner.notes.extend(self.phrase.take_notes());
        Ok(())
    }

    /// Stack `pitches` as a chord at beat `at`, `duration` beats long, with
    /// velocity `gain` (0..1).
    #[pyo3(signature = (pitches, at=0.0, duration=1.0, gain=1.0))]
    fn chord(&mut self, pitches: Vec<String>, at: f32, duration: f32, gain: f32) {
        let refs: Vec<&str> = pitches.iter().map(String::as_str).collect();
        self.phrase.at(at).vel(gain).chord(&refs, duration);
        self.inner.notes.extend(self.phrase.take_notes());
    }

    /// This pattern repeated `times` times end-to-end (`bars × times`), as a
    /// new pattern. `times` 0 is a deliberate silence.
    fn repeat(&self, times: u32) -> Pattern {
        Pattern::from_core(tono_core::song::repeat(&self.inner, STEPS_PER_BAR, times))
    }

    /// This pattern with `other` appended (`other`'s notes start after this
    /// one's bars), as a new pattern of `bars + other.bars`.
    fn concat(&self, other: &Bound<'_, Pattern>) -> Pattern {
        Pattern::from_core(tono_core::song::concat(
            &self.inner,
            &other.borrow().inner,
            STEPS_PER_BAR,
        ))
    }

    /// Both patterns played at once (note sets merged, sorted by step), as a
    /// new pattern as long as the longer of the two.
    fn layer(&self, other: &Bound<'_, Pattern>) -> Pattern {
        Pattern::from_core(tono_core::song::layer(&self.inner, &other.borrow().inner))
    }

    /// The window `[start, start + len)` in STEPS: notes starting inside are
    /// kept and re-based to step 0 (tails may overrun), as a new pattern.
    fn slice(&self, start: u32, len: u32) -> Pattern {
        Pattern::from_core(tono_core::song::slice(
            &self.inner,
            start,
            len,
            STEPS_PER_BAR,
        ))
    }

    /// Every pitch shifted by `semitones` (negative descends), as a new
    /// pattern. Note names come back in canonical sharp spelling; `"midi:N"`
    /// pitches stay `"midi:N"`. An unparseable pitch or a note pushed outside
    /// the MIDI range is a ValueError.
    fn transpose(&self, semitones: i16) -> PyResult<Pattern> {
        tono_core::song::transpose(&self.inner, semitones)
            .map(Pattern::from_core)
            .map_err(|e| PyValueError::new_err(e.to_string()))
    }

    /// Time scaled by exactly `num/den` (2/1 doubles time, 3/2 is a hemiola),
    /// as a new pattern. EXACTLY means exactly: any note landing between grid
    /// steps is a ValueError — nothing is ever rounded silently.
    fn stretch(&self, num: u32, den: u32) -> PyResult<Pattern> {
        tono_core::song::stretch(&self.inner, num, den, STEPS_PER_BAR)
            .map(Pattern::from_core)
            .map_err(|e| PyValueError::new_err(e.to_string()))
    }

    /// Every note's start moved by `shift` steps within the pattern, WRAPPING
    /// around the end, as a new pattern.
    fn rotate(&self, shift: i64) -> Pattern {
        Pattern::from_core(tono_core::song::rotate(&self.inner, shift, STEPS_PER_BAR))
    }

    /// The pattern mirrored in time, as a new pattern.
    fn reverse(&self) -> Pattern {
        Pattern::from_core(tono_core::song::reverse(&self.inner, STEPS_PER_BAR))
    }

    /// Note starts snapped to the nearest multiple of `grid` steps (halves
    /// round forward), as a new pattern; lengths are unchanged.
    fn quantize(&self, grid: u32) -> Pattern {
        Pattern::from_core(tono_core::song::quantize(&self.inner, grid))
    }

    /// Every velocity multiplied by `scale` (clamped to 0..1), as a new
    /// pattern.
    fn vel(&self, scale: f32) -> Pattern {
        Pattern::from_core(tono_core::song::vel(&self.inner, scale))
    }

    /// Every length multiplied by `factor` (a note never vanishes), as a new
    /// pattern — 0.5 is the classic staccato tighten.
    fn gate(&self, factor: f32) -> Pattern {
        Pattern::from_core(tono_core::song::gate(&self.inner, factor))
    }

    /// Deterministic per-note keep/drop: each note survives when its draw
    /// falls under `keep` (0..1). Same pattern + same `seed` ⇒ same drops.
    #[pyo3(signature = (keep, seed=0))]
    fn probability(&self, keep: f32, seed: u64) -> Pattern {
        Pattern::from_core(tono_core::song::probability(&self.inner, keep, seed))
    }

    /// Deterministic per-note jitter BAKED INTO the pattern (structural
    /// humanization — unlike the song/track `humanize` mix knob): timing
    /// shifts up to ±`timing` steps, velocity wobbles up to ±`velocity`. Same
    /// pattern + same `seed` ⇒ same result.
    #[pyo3(signature = (timing=0.0, velocity=0.0, seed=0))]
    fn humanize(&self, timing: f32, velocity: f32, seed: u64) -> Pattern {
        Pattern::from_core(tono_core::song::humanize(
            &self.inner,
            timing,
            velocity,
            seed,
        ))
    }

    fn __repr__(&self) -> String {
        format!(
            "Pattern(bars={}, notes={})",
            self.inner.bars,
            self.inner.notes.len()
        )
    }
}

/// A handle on one of a song's tracks — returned by `Song.track`. Arranging
/// onto it arranges onto the final (slugified, deduplicated) track name, which
/// is also the rendered layer id. The routing methods (`route`, `send`, …)
/// mutate the parent song, which the handle keeps a reference to (one-way —
/// the song never stores handles, so there is no reference cycle).
#[pyclass(module = "tono")]
struct Track {
    name: String,
    song: Py<Song>,
}

#[pymethods]
impl Track {
    /// The track's final name.
    #[getter]
    fn name(&self) -> String {
        self.name.clone()
    }

    /// Route the track's main output to mix bus `bus` (added with
    /// `Song.add_bus`). An unknown bus name is a ValueError listing the
    /// song's buses.
    fn route(&self, py: Python<'_>, bus: &str) -> PyResult<()> {
        let mut song = self.song.borrow_mut(py);
        song.require_bus(bus)?;
        song.track_mut(&self.name).bus = Some(bus.to_string());
        Ok(())
    }

    /// Route the track's main output back to the master bus (the default).
    fn route_master(&self, py: Python<'_>) {
        self.song.borrow_mut(py).track_mut(&self.name).bus = None;
    }

    /// Add a post-fader send from this track to `bus` at `amount` (0..1).
    /// A track sends to a given bus only once — a duplicate target is a
    /// ValueError (remove it with `clear_sends` first).
    #[pyo3(signature = (bus, amount=0.5))]
    fn send(&self, py: Python<'_>, bus: &str, amount: f32) -> PyResult<()> {
        let mut song = self.song.borrow_mut(py);
        song.require_bus(bus)?;
        if !(amount.is_finite() && (0.0..=1.0).contains(&amount)) {
            return Err(PyValueError::new_err(format!(
                "send amount must be in [0, 1], got {amount}"
            )));
        }
        let track = song.track_mut(&self.name);
        if track.sends.iter().any(|s| s.bus == bus) {
            return Err(PyValueError::new_err(format!(
                "track '{}' already sends to bus '{bus}' — clear it with clear_sends, or \
                 adjust the existing send",
                self.name
            )));
        }
        track.sends.push(Send {
            bus: bus.to_string(),
            amount,
        });
        Ok(())
    }

    /// Remove this track's sends: `clear_sends()` removes them all,
    /// `clear_sends("verb")` only the send to that bus.
    #[pyo3(signature = (bus=None))]
    fn clear_sends(&self, py: Python<'_>, bus: Option<&str>) {
        let mut song = self.song.borrow_mut(py);
        let track = song.track_mut(&self.name);
        match bus {
            None => track.sends.clear(),
            Some(b) => track.sends.retain(|s| s.bus != b),
        }
    }

    fn __repr__(&self) -> String {
        format!("Track('{}')", self.name)
    }
}

/// A full song: tracks (catalog voices), patterns (phrases), and an
/// arrangement. `compile` validates and lowers it to an immutable `Program`.
#[pyclass(module = "tono")]
struct Song {
    inner: CoreSong,
    /// Registered patterns, keyed by the Python `id()` of the `Pattern`
    /// object: the first `arrange` of a pattern registers it as `pattern_{n}`
    /// and keeps it alive (so its id can't be recycled) for later placements.
    patterns: HashMap<usize, (Py<Pattern>, String)>,
}

impl Song {
    /// The song track named `name`, mutably — a `Track` handle's name comes
    /// from `Song.track`, so a missing name is a bug, not user error.
    fn track_mut(&mut self, name: &str) -> &mut SongTrack {
        self.inner
            .tracks
            .iter_mut()
            .find(|t| t.name == name)
            .expect("a Track handle outlives its song track")
    }

    /// ValueError unless `bus` names one of the song's buses.
    fn require_bus(&self, bus: &str) -> PyResult<()> {
        if self.inner.buses.iter().any(|b| b.id == bus) {
            return Ok(());
        }
        let known: Vec<&str> = self.inner.buses.iter().map(|b| b.id.as_str()).collect();
        if known.is_empty() {
            Err(PyValueError::new_err(format!(
                "unknown bus {bus:?} — the song has no buses yet (add one with add_bus)"
            )))
        } else {
            Err(PyValueError::new_err(format!(
                "unknown bus {bus:?} — the song's buses are: {}",
                known.join(", ")
            )))
        }
    }
}

#[pymethods]
impl Song {
    /// A song named `name` at `tempo` BPM. `seed` pins the deterministic RNG
    /// stream everything stochastic draws from (None = the document default,
    /// 0): same song + same seed ⇒ same program hash.
    #[new]
    #[pyo3(signature = (name, tempo=120.0, seed=None))]
    fn new(name: &str, tempo: f32, seed: Option<u64>) -> Self {
        let mut inner = CoreSong::new(name, tempo);
        if let Some(seed) = seed {
            inner = inner.with_seed(seed);
        }
        Song {
            inner,
            patterns: HashMap::new(),
        }
    }

    /// The song's name.
    #[getter]
    fn name(&self) -> String {
        self.inner.name.clone()
    }

    /// The tempo in beats per minute.
    #[getter]
    fn tempo(&self) -> f32 {
        self.inner.bpm
    }

    /// The track names, in declaration order.
    #[getter]
    fn track_names(&self) -> Vec<String> {
        self.inner.tracks.iter().map(|t| t.name.clone()).collect()
    }

    /// Add a track playing `voice` under `name` (slugified and deduplicated —
    /// the final name keeps layer ids stable across faces) and return its
    /// `Track` handle. Notes come from the patterns arranged onto it.
    fn track(slf: &Bound<'_, Song>, name: &str, voice: PyRef<'_, Voice>) -> Track {
        let final_name = {
            let mut song = slf.borrow_mut();
            song.inner.add_voice(name, &voice.inner);
            song.inner
                .tracks
                .last()
                .expect("add_voice just pushed a track")
                .name
                .clone()
        };
        Track {
            name: final_name,
            song: slf.clone().unbind(),
        }
    }

    /// Arrange `pattern` onto `track` (a `Track` handle or a track name).
    /// `bars` is an int (place at that one bar) or a range/list of ints
    /// (place at each). The first arrange of a pattern registers it under
    /// `pattern_{n}` (1-based, in registration order). With a meter map or
    /// pickup set, a bar that lands between grid steps is a ValueError here
    /// (rather than a compile error later).
    #[pyo3(signature = (track, pattern, bars=None))]
    fn arrange(
        &mut self,
        track: &Bound<'_, PyAny>,
        pattern: &Bound<'_, Pattern>,
        bars: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<()> {
        let track_name = track_name_of(track)?;
        let bars: Vec<u32> = match bars {
            None => vec![0],
            Some(obj) => {
                if let Ok(bar) = obj.extract::<u32>() {
                    vec![bar]
                } else if let Ok(list) = obj.extract::<Vec<u32>>() {
                    list
                } else {
                    return Err(PyValueError::new_err(
                        "bars must be an int or a range/list of ints",
                    ));
                }
            }
        };

        // With a meter map or pickup, bars move through the exact beat walk —
        // check every placement lands on the grid BEFORE mutating anything
        // (the compiler's T1005 catches maps set after the arrange).
        if !(self.inner.meter_map.is_empty() && self.inner.pickup.is_none()) {
            let spb = i128::from(self.inner.steps_per_beat.max(1));
            for &bar in &bars {
                let beat = self.inner.beat_at_bar(bar);
                if (beat.num as i128 * spb) % i128::from(beat.den) != 0 {
                    return Err(PyValueError::new_err(format!(
                        "bar {bar} lands at beat {beat}, between grid steps ({} steps per \
                         beat) — raise steps_per_beat, or move the placement/meter change \
                         onto the grid",
                        self.inner.steps_per_beat.max(1)
                    )));
                }
            }
        }

        let key = pattern.as_ptr() as usize;
        let registered = match self.patterns.get(&key) {
            Some((_, name)) => name.clone(),
            None => {
                // 1-based count of registered patterns, skipping names a
                // loaded (from_json) song already carries.
                let mut n = self.patterns.len() + 1;
                let mut name = format!("pattern_{n}");
                while self.inner.patterns.iter().any(|p| p.name == name) {
                    n += 1;
                    name = format!("pattern_{n}");
                }
                let (bars_len, notes) = {
                    let p = pattern.borrow();
                    (p.inner.bars, p.inner.notes.clone())
                };
                self.inner.add_pattern(&name, bars_len, notes);
                self.patterns
                    .insert(key, (pattern.clone().unbind(), name.clone()));
                name
            }
        };
        for bar in bars {
            self.inner.arrange(&track_name, &registered, bar);
        }
        Ok(())
    }

    /// Set the tempo map: `[(beat, bpm), ...]` — tempo changes at exact beat
    /// positions (each beat accepts int / float / Fraction / (num, den); a
    /// float is its EXACT binary value, so 0.1 isn't 1/10 — use Fraction for
    /// exact decimals). Validated eagerly against the compiler's rules
    /// (ValueError naming the problem): the first point must sit at beat 0,
    /// beats strictly ascend, tempos are positive and finite, at most 1024
    /// points. `set_tempo_map([])` clears the map (constant tempo).
    fn set_tempo_map(&mut self, points: Vec<(Bound<'_, PyAny>, f32)>) -> PyResult<()> {
        if points.len() > 1024 {
            return Err(PyValueError::new_err(format!(
                "tempo_map is capped at 1024 points, got {}",
                points.len()
            )));
        }
        let mut map = Vec::with_capacity(points.len());
        for (i, (beat, bpm)) in points.iter().enumerate() {
            let at = py_beat(beat)?;
            if !(bpm.is_finite() && *bpm > 0.0) {
                return Err(PyValueError::new_err(format!(
                    "tempo_map[{i}].bpm: tempo must be positive and finite, got {bpm}"
                )));
            }
            map.push(TempoPoint { at, bpm: *bpm });
        }
        if let Some(first) = map.first()
            && first.at != Beat::zero()
        {
            return Err(PyValueError::new_err(
                "tempo_map's first point must be at beat 0 (the song's tempo applies before \
                 the first change otherwise)",
            ));
        }
        for (i, w) in map.windows(2).enumerate() {
            if w[1].at <= w[0].at {
                return Err(PyValueError::new_err(format!(
                    "tempo_map[{}].at: tempo_map must be strictly ascending by beat",
                    i + 1
                )));
            }
        }
        self.inner.tempo_map = map;
        Ok(())
    }

    /// Set the meter map: `[(bar, numerator, denominator), ...]` — the time
    /// signature from each bar on (6/8 is `(bar, 6, 8)`). Validated eagerly
    /// against the compiler's rules (ValueError naming the problem): the
    /// first point must be bar 0, bars strictly ascend, numerator ≥ 1,
    /// denominator a power of two ≤ 64, at most 256 points.
    /// `set_meter_map([])` clears the map (the song's default 4/4).
    fn set_meter_map(&mut self, points: Vec<(u32, u32, u32)>) -> PyResult<()> {
        if points.len() > 256 {
            return Err(PyValueError::new_err(format!(
                "meter_map is capped at 256 points, got {}",
                points.len()
            )));
        }
        for (i, (bar, numerator, denominator)) in points.iter().enumerate() {
            if *numerator < 1 {
                return Err(PyValueError::new_err(format!(
                    "meter_map[{i}].numerator: time-signature numerator must be ≥ 1"
                )));
            }
            if !denominator.is_power_of_two() || *denominator > 64 {
                return Err(PyValueError::new_err(format!(
                    "meter_map[{i}].denominator: time-signature denominator must be a \
                     power of two ≤ 64, got {denominator}"
                )));
            }
            if i > 0 && *bar <= points[i - 1].0 {
                return Err(PyValueError::new_err(format!(
                    "meter_map[{i}].bar: meter_map must be strictly ascending by bar"
                )));
            }
        }
        if let Some(first) = points.first()
            && first.0 != 0
        {
            return Err(PyValueError::new_err(
                "meter_map's first point must be at bar 0 (add the opening time signature \
                 at bar 0)",
            ));
        }
        self.inner.meter_map = points
            .iter()
            .map(|&(bar, numerator, denominator)| MeterPoint {
                bar,
                numerator,
                denominator,
            })
            .collect();
        Ok(())
    }

    /// Set the pickup (anacrusis): bar 0's length in beats when it isn't a
    /// full bar (same beat forms as the tempo map — e.g.
    /// `set_pickup(Fraction(1, 2))` for an eighth-note pickup in 4/4).
    /// Negative is a ValueError.
    fn set_pickup(&mut self, beat: &Bound<'_, PyAny>) -> PyResult<()> {
        let beat = py_beat(beat)?;
        if beat < Beat::zero() {
            return Err(PyValueError::new_err("the pickup bar can't be negative"));
        }
        self.inner.pickup = Some(beat);
        Ok(())
    }

    /// Clear the pickup — bar 0 is full length again.
    fn clear_pickup(&mut self) {
        self.inner.pickup = None;
    }

    /// Add a named section (`name` starting at `bar`, `bars` long) — musical
    /// metadata compiled into the Program for the runtime's quantized
    /// transitions. Empty names and zero-length sections are ValueErrors.
    fn add_section(&mut self, name: &str, bar: u32, bars: u32) -> PyResult<()> {
        if name.is_empty() {
            return Err(PyValueError::new_err(
                "a section needs a name (e.g. \"verse\", \"chorus\")",
            ));
        }
        if bars < 1 {
            return Err(PyValueError::new_err("a section must be at least one bar"));
        }
        self.inner.sections.push(Section {
            name: name.to_string(),
            bar,
            bars,
        });
        Ok(())
    }

    /// Add a named marker at an exact beat (same beat forms as the tempo
    /// map) — metadata compiled into the Program. Empty names are ValueErrors.
    fn add_marker(&mut self, name: &str, beat: &Bound<'_, PyAny>) -> PyResult<()> {
        if name.is_empty() {
            return Err(PyValueError::new_err(
                "a marker needs a name (e.g. \"drop\", \"cue\")",
            ));
        }
        self.inner.markers.push(Marker {
            name: name.to_string(),
            at: py_beat(beat)?,
        });
        Ok(())
    }

    /// Add a mix bus: a named submix tracks route to (`Track.route`) and feed
    /// (`Track.send`), with an insert chain of `effects` — a list of
    /// `(type, params)` tuples like `("reverb", {"room": 0.5, "mix": 0.3})`.
    /// The id is a short slug (a-z, 0-9, _), unique and never `"master"`;
    /// `gain` is the return fader (0..2). Unknown/non-processor effect types
    /// and unknown params are ValueErrors naming the accepted forms.
    #[pyo3(signature = (id, gain=1.0, effects=None))]
    fn add_bus(
        &mut self,
        id: &str,
        gain: f32,
        effects: Option<Vec<(String, Bound<'_, PyAny>)>>,
    ) -> PyResult<()> {
        if id.is_empty()
            || !id
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
        {
            return Err(PyValueError::new_err(format!(
                "bus ids are short slugs (a-z, 0-9, _), got '{id}'"
            )));
        }
        if id == "master" {
            return Err(PyValueError::new_err(
                "'master' is reserved for the master chain; pick another bus id",
            ));
        }
        if self.inner.buses.iter().any(|b| b.id == id) {
            return Err(PyValueError::new_err(format!(
                "duplicate bus id '{id}' — ids must be unique"
            )));
        }
        if self.inner.tracks.iter().any(|t| t.name == id) {
            return Err(PyValueError::new_err(format!(
                "bus id '{id}' is also a track name — a track and its bus must be named apart"
            )));
        }
        if !(gain.is_finite() && (0.0..=2.0).contains(&gain)) {
            return Err(PyValueError::new_err(format!(
                "bus '{id}': gain must be in [0, 2], got {gain}"
            )));
        }
        let mut chain = Vec::new();
        for (kind, params) in effects.unwrap_or_default() {
            chain.push(build_effect(&kind, &params)?);
        }
        self.inner.buses.push(Bus {
            id: id.to_string(),
            gain,
            effects: chain,
        });
        Ok(())
    }

    /// Automate a track's `target` (`"gain"` or `"pan"`) with beat-addressed
    /// breakpoints `[(beat, value), ...]` (floats; compiled to seconds through
    /// the tempo map). `curve` is `"linear"` (default), `"step"`, or `"exp"`.
    /// REPLACES any existing lane for that target on the track; the other
    /// target's lane is kept. An unknown track is a ValueError naming the
    /// song's tracks.
    #[pyo3(signature = (track, target, points, curve="linear"))]
    fn automate(
        &mut self,
        track: &Bound<'_, PyAny>,
        target: &str,
        points: Vec<(f32, f32)>,
        curve: &str,
    ) -> PyResult<()> {
        let name = track_name_of(track)?;
        let target = match target {
            "gain" => AutoTarget::Gain,
            "pan" => AutoTarget::Pan,
            other => {
                return Err(PyValueError::new_err(format!(
                    "unknown automation target {other:?} — expected 'gain' or 'pan'"
                )));
            }
        };
        let curve = match curve {
            "linear" => AutoCurve::Linear,
            "step" => AutoCurve::Step,
            "exp" => AutoCurve::Exp,
            other => {
                return Err(PyValueError::new_err(format!(
                    "unknown automation curve {other:?} — expected one of: linear, step, exp"
                )));
            }
        };
        if !self.inner.tracks.iter().any(|t| t.name == name) {
            let known: Vec<&str> = self.inner.tracks.iter().map(|t| t.name.as_str()).collect();
            return Err(PyValueError::new_err(format!(
                "unknown track {name:?} — the song's tracks are: {}",
                known.join(", ")
            )));
        }
        let track = self.track_mut(&name);
        track.automation.retain(|lane| lane.target != target);
        track.automation.push(SongLane {
            target,
            curve,
            points: points
                .into_iter()
                .map(|(at, v)| SongPoint { at, v })
                .collect(),
        });
        Ok(())
    }

    /// Compile the song to an immutable `Program` — validation collects every
    /// problem in one pass and failures raise `tono.CompileError` carrying the
    /// structured `.diagnostics`. `target` is `"offline"` (where streaming
    /// blockers are warnings) or `"runtime"` (where they are errors).
    #[pyo3(signature = (sample_rate=None, target="offline"))]
    fn compile(&self, py: Python<'_>, sample_rate: Option<u32>, target: &str) -> PyResult<Program> {
        let target = match target {
            "offline" => CompileTarget::Offline,
            "runtime" => CompileTarget::Runtime,
            other => {
                return Err(PyValueError::new_err(format!(
                    "unknown compile target {other:?} — expected 'offline' or 'runtime'"
                )));
            }
        };
        let opts = CompileOptions {
            sample_rate,
            target,
        };
        match py.detach(|| self.inner.compile(&opts)) {
            Ok(program) => Ok(Program {
                inner: Arc::new(program),
            }),
            Err(err) => Err(compile_error(py, err)),
        }
    }

    /// Serialize the song (its saveable project form).
    fn to_json(&self) -> String {
        serde_json::to_string(&self.inner).expect("a song serializes")
    }

    /// Load a song saved with `to_json`.
    #[classmethod]
    fn from_json(_cls: &Bound<'_, PyType>, json: &str) -> PyResult<Self> {
        let inner: CoreSong =
            serde_json::from_str(json).map_err(|e| PyValueError::new_err(e.to_string()))?;
        Ok(Song {
            inner,
            patterns: HashMap::new(),
        })
    }

    fn __repr__(&self) -> String {
        format!(
            "Song('{}', tempo={:?}, tracks={})",
            self.inner.name,
            self.inner.bpm,
            self.inner.tracks.len()
        )
    }
}

/// A compiled song: validated, resolved, hashed — the immutable artifact
/// applications render and ship.
#[pyclass(module = "tono")]
pub(crate) struct Program {
    /// Shared: a `Performance` Arc-clones the program as its running (and
    /// swap-target) artifact.
    inner: Arc<CoreProgram>,
}

impl Program {
    /// The shared inner program, for `Performance::new` / `swap_to`.
    pub(crate) fn shared(&self) -> Arc<CoreProgram> {
        self.inner.clone()
    }
}

#[pymethods]
impl Program {
    /// The canonical content hash: equivalent songs hash equal, from Rust or
    /// Python alike.
    #[getter]
    fn hash(&self) -> u64 {
        self.inner.hash
    }

    /// The sample rate the program was compiled for.
    #[getter]
    fn sample_rate(&self) -> u32 {
        self.inner.doc.sample_rate
    }

    /// The total duration in seconds, including the release/reverb tail.
    #[getter]
    fn duration_seconds(&self) -> f32 {
        self.inner.doc.duration
    }

    /// One dict per track, in declaration order: index, name, wave, notes, mute,
    /// solo.
    #[getter]
    fn tracks<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyList>> {
        let list = PyList::empty(py);
        for (index, t) in self.inner.meta.tracks.iter().enumerate() {
            let dict = PyDict::new(py);
            dict.set_item("index", index)?;
            dict.set_item("name", &t.name)?;
            dict.set_item("wave", wave_str(t.wave))?;
            dict.set_item("notes", t.notes)?;
            dict.set_item("mute", t.mute)?;
            dict.set_item("solo", t.solo)?;
            list.append(dict)?;
        }
        Ok(list)
    }

    /// Bounded estimates of what the program costs to render or run:
    /// frames, events, peak_voices, memory_bytes.
    #[getter]
    fn estimates<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyDict>> {
        let dict = PyDict::new(py);
        dict.set_item("frames", self.inner.estimates.frames)?;
        dict.set_item("events", self.inner.estimates.events)?;
        dict.set_item("peak_voices", self.inner.estimates.peak_voices)?;
        dict.set_item("memory_bytes", self.inner.estimates.memory_bytes)?;
        Ok(dict)
    }

    /// Offline compile warnings (the streaming blockers), as diagnostic dicts
    /// of the same shape as `CompileError.diagnostics`.
    #[getter]
    fn warnings<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyList>> {
        diagnostics_list(py, &self.inner.warnings())
    }

    /// Whether the resolved document streams natively.
    #[getter]
    fn is_streamable(&self) -> bool {
        self.inner.is_streamable()
    }

    /// Render the full program to a stereo `np.ndarray` of shape
    /// `(frames, 2)`, dtype `float32`, C-order, channel order L/R. The array
    /// is an owned copy: safe to keep or mutate, and rendering again yields
    /// the same bytes.
    fn render<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyArray2<f32>>> {
        let (left, right) = py.detach(|| self.inner.render_stereo());
        let frames = left.len();
        let mut interleaved = Vec::with_capacity(frames * 2);
        for i in 0..frames {
            interleaved.push(left[i]);
            interleaved.push(right[i]);
        }
        interleaved.into_pyarray(py).reshape([frames, 2])
    }

    /// Render the full program to a mono `np.ndarray` of shape `(frames,)`,
    /// dtype `float32` — the mid of the stereo render, an owned copy.
    fn render_mono<'py>(&self, py: Python<'py>) -> Bound<'py, PyArray1<f32>> {
        py.detach(|| self.inner.render_mono()).into_pyarray(py)
    }

    /// Render a selected range as a stereo `(frames, 2)` float32 array — a
    /// slice of the full render, so tails crossing the boundary sound
    /// exactly as in the full mix. Pass `frames=(start, end)` or
    /// `bars=(start_bar, end_bar)` (through the program's meter map).
    #[pyo3(signature = (*, frames=None, bars=None))]
    fn render_range<'py>(
        &self,
        py: Python<'py>,
        frames: Option<(u64, u64)>,
        bars: Option<(u32, u32)>,
    ) -> PyResult<Bound<'py, PyArray2<f32>>> {
        match (frames, bars) {
            (Some((s, e)), None) => {
                let (l, r) = py.detach(|| self.inner.render_range_frames(s, e));
                let n = l.len();
                let mut interleaved = Vec::with_capacity(n * 2);
                for i in 0..n {
                    interleaved.push(l[i]);
                    interleaved.push(r[i]);
                }
                interleaved.into_pyarray(py).reshape([n, 2])
            }
            (None, Some((s, e))) => {
                let (l, r) = py.detach(|| self.inner.render_range_bars(s, e));
                let n = l.len();
                let mut interleaved = Vec::with_capacity(n * 2);
                for i in 0..n {
                    interleaved.push(l[i]);
                    interleaved.push(r[i]);
                }
                interleaved.into_pyarray(py).reshape([n, 2])
            }
            _ => Err(PyValueError::new_err(
                "pass exactly one of frames=(start, end) or bars=(start_bar, end_bar)",
            )),
        }
    }

    /// Render per-track and per-bus stereo stems (pre-master-chain): a dict
    /// mapping stem id (`"bass"`, `"bus:verb"`) to an `(frames, 2)` float32
    /// array, in declaration order. A stem's `bus` routing is in
    /// `stem_routing`. Muted tracks are silent stems.
    fn render_stems<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyDict>> {
        let stems = py.detach(|| self.inner.render_stems());
        let dict = PyDict::new(py);
        for s in &stems {
            let frames = s.left.len();
            let mut interleaved = Vec::with_capacity(frames * 2);
            for i in 0..frames {
                interleaved.push(s.left[i]);
                interleaved.push(s.right[i]);
            }
            dict.set_item(&s.id, interleaved.into_pyarray(py).reshape([frames, 2])?)?;
        }
        Ok(dict)
    }

    /// Where each track stem routes: `{track_id: bus_id}` for tracks whose
    /// main output goes to a bus (their stem is already inside that bus's
    /// stem). Read from the compiled document — no render pass.
    #[getter]
    fn stem_routing<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyDict>> {
        let dict = PyDict::new(py);
        if let tono_core::dsl::Node::Tracks { tracks, .. } = &self.inner.doc.root {
            for t in tracks {
                if let (Some(id), Some(bus)) = (&t.id, &t.bus) {
                    dict.set_item(id, bus)?;
                }
            }
        }
        Ok(dict)
    }

    /// Serialize the program bundle (compact JSON).
    fn to_json(&self) -> String {
        self.inner.to_json()
    }

    /// Load a program bundle: rejects bundles newer than this binary (T3001)
    /// and re-verifies the content hash (T3002).
    #[staticmethod]
    fn from_json(json: &str) -> PyResult<Self> {
        CoreProgram::from_json(json)
            .map(|inner| Program {
                inner: Arc::new(inner),
            })
            .map_err(|e| TonoError::new_err(e.to_string()))
    }

    /// Write the program bundle to `path` (see `to_json`).
    fn save(&self, path: &str) -> PyResult<()> {
        std::fs::write(path, self.inner.to_json()).map_err(|e| PyOSError::new_err(e.to_string()))
    }

    /// Read a program bundle from `path` (see `from_json`).
    #[staticmethod]
    fn load(path: &str) -> PyResult<Self> {
        let json = std::fs::read_to_string(path).map_err(|e| PyOSError::new_err(e.to_string()))?;
        Program::from_json(&json)
    }

    fn __repr__(&self) -> String {
        format!(
            "Program('{}', hash={:#018x}, tracks={})",
            self.inner.doc.name,
            self.inner.hash,
            self.inner.meta.tracks.len()
        )
    }
}

/// Register the typed-API classes, exceptions, and the `instruments`
/// submodule on the extension module.
pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add("TonoError", m.py().get_type::<TonoError>())?;
    m.add("CompileError", m.py().get_type::<CompileError>())?;
    m.add_class::<Voice>()?;
    m.add_class::<Pattern>()?;
    m.add_class::<Track>()?;
    m.add_class::<Song>()?;
    m.add_class::<Program>()?;

    let instruments = PyModule::new(m.py(), "instruments")?;
    instruments.add_function(wrap_pyfunction!(piano, &instruments)?)?;
    instruments.add_function(wrap_pyfunction!(electric_piano, &instruments)?)?;
    instruments.add_function(wrap_pyfunction!(organ, &instruments)?)?;
    instruments.add_function(wrap_pyfunction!(strings, &instruments)?)?;
    instruments.add_function(wrap_pyfunction!(bass, &instruments)?)?;
    instruments.add_function(wrap_pyfunction!(guitar, &instruments)?)?;
    instruments.add_function(wrap_pyfunction!(drums, &instruments)?)?;
    instruments.add_function(wrap_pyfunction!(brass, &instruments)?)?;
    instruments.add_function(wrap_pyfunction!(flute, &instruments)?)?;
    instruments.add_function(wrap_pyfunction!(mallets, &instruments)?)?;
    instruments.add_function(wrap_pyfunction!(bells, &instruments)?)?;
    m.add_submodule(&instruments)?;
    Ok(())
}
