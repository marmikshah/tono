//! `tono.Pattern`: a phrase on the typed API's 16-steps-per-bar grid with
//! pure transform methods.

use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;
use pyo3::types::PyType;

use tono_core::song::{Pattern as CorePattern, Phrase};

/// The typed API's grid: 4 steps per beat (sixteenth notes) — the same
/// `steps_per_beat` `Song::new` stamps, shared by the pattern write cursor and
/// every pattern op below.
const STEPS_PER_BEAT: u32 = 4;
/// The default meter's beats per bar (`Song::new`'s `beats_per_bar`).
const BEATS_PER_BAR: u32 = 4;
/// Steps per bar on that grid — the `steps_per_bar` argument every pattern op
/// (`repeat`, `concat`, `slice`, `rotate`, `reverse`, `euclidean`) runs on.
const STEPS_PER_BAR: u32 = STEPS_PER_BEAT * BEATS_PER_BAR;

/// A reusable musical phrase, `bars` long, on the typed API's grid of 4 steps
/// per beat / 16 steps per bar (songs created through this API keep that
/// default grid). Notes are placed by beat (floats welcome: beat 0.5 is the
/// second eighth note) and snap to the grid exactly like the Rust `Phrase`
/// writer. The transform methods (`transpose`, `stretch`, `reverse`, …) are
/// pure: each returns a NEW pattern and never mutates this one.
#[pyclass(module = "tono")]
pub(crate) struct Pattern {
    /// The core pattern (name/bars/notes): the value the ops consume and the
    /// arrangement registers. Owns the pattern's notes.
    pub(crate) inner: CorePattern,
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

/// Register `Pattern`.
pub(crate) fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<Pattern>()
}
