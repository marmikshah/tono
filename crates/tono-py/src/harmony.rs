//! `tono.Pitch` / `tono.Key` / `tono.Chord` — thin, query-only wrappers over
//! tono-core's `music` module: the strict-grammar harmony vocabulary (one
//! spelling grammar, no guessing — a misspelled name is a ValueError naming
//! the valid forms). Pitches compare equal enharmonically (by MIDI number).
//!

use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;

use tono_core::music::{Chord as CoreChord, Key as CoreKey, Pitch as CorePitch};

/// Map a `MusicError` to a Python `ValueError` (its message already names the
/// rejected input and the valid grammar).
fn music_err(e: tono_core::music::MusicError) -> PyErr {
    PyValueError::new_err(e.to_string())
}

/// An absolute pitch: a note name (`"C4"`, `"F#3"`, `"Gb5"` — enharmonics
/// collapse to the canonical sharp spelling) or `"midi:N"`. Middle C is
/// `"C4"` (MIDI 60).
#[pyclass(module = "tono")]
struct Pitch {
    inner: CorePitch,
}

impl From<CorePitch> for Pitch {
    fn from(inner: CorePitch) -> Self {
        Pitch { inner }
    }
}

#[pymethods]
impl Pitch {
    /// Parse a pitch name, strictly (the octave is mandatory, accidentals
    /// single). A bad name is a ValueError naming the valid forms.
    #[new]
    fn new(name: &str) -> PyResult<Self> {
        CorePitch::from_name(name)
            .map(Pitch::from)
            .map_err(music_err)
    }

    /// The MIDI note number, 0..=127.
    #[getter]
    fn midi(&self) -> u8 {
        self.inner.to_midi()
    }

    /// The canonical name (sharp spelling plus octave, e.g. `"F#3"`).
    #[getter]
    fn name(&self) -> String {
        self.inner.to_string()
    }

    /// The pitch `semitones` away (negative descends), as a new Pitch.
    /// Landing outside the MIDI range is a ValueError — transposition never
    /// wraps or clamps.
    fn transpose(&self, semitones: i16) -> PyResult<Pitch> {
        self.inner
            .add_semitones(semitones)
            .map(Pitch::from)
            .map_err(music_err)
    }

    /// Equality is by MIDI number — enharmonic spellings compare equal
    /// (`Pitch("Gb3") == Pitch("F#3")`).
    fn __eq__(&self, other: &Bound<'_, PyAny>) -> bool {
        other
            .cast::<Pitch>()
            .map(|p| p.borrow().inner == self.inner)
            .unwrap_or(false)
    }

    fn __str__(&self) -> String {
        self.inner.to_string()
    }

    fn __repr__(&self) -> String {
        format!("Pitch('{}')", self.inner)
    }
}

/// A key: a scale rooted at a tonic — the thing `"C major"` names. Parses
/// `"<tonic> <scale>"` (`"C major"`, `"A minor"`, `"F# dorian"`).
#[pyclass(module = "tono")]
struct Key {
    inner: CoreKey,
}

#[pymethods]
impl Key {
    /// Parse a key name, strictly (one tonic, one space, one scale name). A
    /// bad name is a ValueError naming the valid forms.
    #[new]
    fn new(name: &str) -> PyResult<Self> {
        CoreKey::from_name(name)
            .map(|inner| Key { inner })
            .map_err(music_err)
    }

    /// The canonical name (sharp tonic plus scale, e.g. `"C major"`).
    #[getter]
    fn name(&self) -> String {
        self.inner.to_string()
    }

    /// The pitch of scale degree `n` (1-based: 1 = tonic) in `octave` —
    /// degree 3 of C major at octave 4 is E4, and degrees past the scale
    /// length cross into the next octave. Degree 0 is a ValueError.
    #[pyo3(signature = (n, octave=4))]
    fn degree(&self, n: u32, octave: i8) -> PyResult<Pitch> {
        self.inner
            .degree_pitch(n, octave)
            .map(Pitch::from)
            .map_err(music_err)
    }

    /// Does `pitch` (a `tono.Pitch`) belong to the key?
    fn contains(&self, pitch: PyRef<'_, Pitch>) -> bool {
        self.inner.contains(pitch.inner)
    }

    fn __str__(&self) -> String {
        self.inner.to_string()
    }

    fn __repr__(&self) -> String {
        format!("Key('{}')", self.inner)
    }
}

/// A chord: a root plus a quality — `"C"`, `"Cm"`, `"Cmaj7"`, `"Cm7"`,
/// `"C7"`, `"Cdim"`, `"Caug"`. Query-only: notes, voicings, inversions.
#[pyclass(module = "tono")]
struct Chord {
    inner: CoreChord,
}

#[pymethods]
impl Chord {
    /// Parse a chord name, strictly (a root plus one of "", "m", "maj7",
    /// "m7", "7", "dim", "aug"). A bad name is a ValueError naming the valid
    /// forms.
    #[new]
    fn new(name: &str) -> PyResult<Self> {
        CoreChord::from_name(name)
            .map(|inner| Chord { inner })
            .map_err(music_err)
    }

    /// The canonical name (sharp root plus quality suffix, e.g. `"Cm7"`).
    #[getter]
    fn name(&self) -> String {
        self.inner.to_string()
    }

    /// The chord tones as pitch-class names (no octave), ascending from the
    /// root — `Chord("C").notes()` is `["C", "E", "G"]`.
    fn notes(&self) -> Vec<String> {
        self.inner.notes().iter().map(|pc| pc.to_string()).collect()
    }

    /// The root-position close voicing in `octave`, ascending (`Chord("C")`
    /// at octave 4 is C4 E4 G4). Running past the MIDI range is a ValueError.
    #[pyo3(signature = (octave=4))]
    fn pitches(&self, octave: i8) -> PyResult<Vec<Pitch>> {
        self.inner
            .arp(octave)
            .map(|ps| ps.into_iter().map(Pitch::from).collect())
            .map_err(music_err)
    }

    /// The `n`-th inversion as a voicing in `octave`: the lowest `n` voices
    /// raised an octave, so the `n`-th chord tone sits in the bass
    /// (`Chord("C").invert(1)` is E4 G4 C5). `n` wraps modulo the chord size.
    #[pyo3(signature = (n=1, octave=4))]
    fn invert(&self, n: u32, octave: i8) -> PyResult<Vec<Pitch>> {
        self.inner
            .invert(n)
            .pitches(octave)
            .map(|ps| ps.into_iter().map(Pitch::from).collect())
            .map_err(music_err)
    }

    /// The ascending arpeggio from the root in `octave` — the same pitches as
    /// `pitches()` (root position).
    #[pyo3(signature = (octave=4))]
    fn arp(&self, octave: i8) -> PyResult<Vec<Pitch>> {
        self.pitches(octave)
    }

    fn __str__(&self) -> String {
        self.inner.to_string()
    }

    fn __repr__(&self) -> String {
        format!("Chord('{}')", self.inner)
    }
}

/// Register the harmony classes on the extension module.
pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<Pitch>()?;
    m.add_class::<Key>()?;
    m.add_class::<Chord>()?;
    Ok(())
}
