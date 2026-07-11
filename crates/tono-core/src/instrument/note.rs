//! Note names and the instrument error type.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

use crate::dsl::note_to_hz;

/// Why an [`Instrument`](super::Instrument) could not be built.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum InstrumentError {
    /// The patch's graph is outside the streaming subset, so it can't play in
    /// real time (e.g. a `tracks` root, a `normalize` stage, or a sampler seq).
    NotStreamable,
    /// The patch failed to instantiate at its defaults (a bad param path/value).
    BadPatch(String),
}

impl fmt::Display for InstrumentError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            InstrumentError::NotStreamable => {
                write!(
                    f,
                    "instrument patch is not streamable (can't play in real time)"
                )
            }
            InstrumentError::BadPatch(e) => write!(f, "instrument patch is invalid: {e}"),
        }
    }
}

impl std::error::Error for InstrumentError {}

/// A musical pitch as a MIDI note number (0–127). `A4` = 69 = 440 Hz.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Note(pub u8);

impl Note {
    /// Middle C.
    pub const C4: Note = Note(60);
    /// Concert A (440 Hz).
    pub const A4: Note = Note(69);

    /// The note's frequency in Hz (equal temperament, A4 = 440).
    pub fn freq(self) -> f32 {
        440.0 * 2f32.powf((self.0 as f32 - 69.0) / 12.0)
    }

    /// The MIDI note number.
    pub fn midi(self) -> u8 {
        self.0
    }

    /// Parse a note name (`"C4"`, `"F#3"`, `"Bb5"`) or `"midi:60"` into the
    /// nearest MIDI note.
    pub fn parse(s: &str) -> Option<Note> {
        let hz = note_to_hz(s)?;
        let midi = (69.0 + 12.0 * (hz / 440.0).log2()).round();
        if (0.0..=127.0).contains(&midi) {
            Some(Note(midi as u8))
        } else {
            None
        }
    }

    /// Shift by `semitones` (clamped to the MIDI range).
    pub fn transpose(self, semitones: i32) -> Note {
        Note((self.0 as i32 + semitones).clamp(0, 127) as u8)
    }
}

impl fmt::Display for Note {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        const NAMES: [&str; 12] = [
            "C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B",
        ];
        write!(
            f,
            "{}{}",
            NAMES[(self.0 % 12) as usize],
            self.0 as i32 / 12 - 1
        )
    }
}

impl FromStr for Note {
    type Err = ();
    fn from_str(s: &str) -> Result<Note, ()> {
        Note::parse(s).ok_or(())
    }
}

impl From<u8> for Note {
    fn from(midi: u8) -> Note {
        Note(midi)
    }
}
