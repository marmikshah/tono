//! music — harmony: pitches, intervals, scales, keys, chords, and voicings.
//!
//! The shared vocabulary for talking about WHICH notes play, one layer above
//! the raw graph: a [`Pitch`] is an absolute note (a MIDI number), a
//! [`PitchClass`] a note name without an octave, an [`Interval`] a distance
//! in semitones, and [`Scale`], [`Key`], [`Chord`], and [`Voicing`] build
//! upward from there. Everything is exact integer arithmetic; the only float
//! in the module is [`Pitch::to_hz`], the single boundary where harmony
//! becomes audio — it uses the same formula as the DSL's
//! [`note_to_hz`](crate::dsl::note_to_hz) (12-TET, A4 = 440), so a pitch
//! spelled `"F#3"` here means exactly the frequency a `Value::Note` resolves
//! to at render.
//!
//! Name parsing follows the DSL's spelling convention (`"C4"`, `"F#3"`,
//! `"Gb5"`, `"midi:60"`) as a STRICT subset: one grammar, no guessing. A
//! misspelled name is an error naming the valid forms — never a silent
//! correction (so the DSL's leniency — a defaulted octave, repeated
//! accidentals, the `"m69"` shorthand — is NOT accepted here).
//!
//! ```
//! use tono_core::music::{Chord, Key, Pitch};
//!
//! let key = Key::from_name("C major").unwrap();
//! assert!(key.contains(Pitch::from_name("E4").unwrap()));
//!
//! let g7 = Chord::from_name("G7").unwrap();
//! let arp: Vec<String> = g7.arp(4).unwrap().iter().map(ToString::to_string).collect();
//! assert_eq!(arp, ["G4", "B4", "D5", "F5"]);
//! ```
//!

use serde::{Deserialize, Serialize};

/// The semitone base of a note letter, A–G in either case (`C` = 0).
fn letter_semitones(byte: u8) -> Option<i16> {
    match byte.to_ascii_uppercase() {
        b'C' => Some(0),
        b'D' => Some(2),
        b'E' => Some(4),
        b'F' => Some(5),
        b'G' => Some(7),
        b'A' => Some(9),
        b'B' => Some(11),
        _ => None,
    }
}

/// A chromatic pitch class: a note name without an octave, as a semitone
/// number 0..=11 where C = 0. Enharmonic spellings collapse — `"F#"` and
/// `"Gb"` are the same class; the sharp spelling is canonical for display.
///
/// Serde is the bare semitone number (`6`); deserializing REJECTS values
/// above 11 rather than wrapping (serde is the strict path —
/// [`PitchClass::from_semitone`] is the permissive one).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PitchClass(u8);

impl PitchClass {
    /// The class `semitone` steps above C, wrapping modulo 12 (13 → C♯ = 1).
    pub const fn from_semitone(semitone: u8) -> PitchClass {
        PitchClass(semitone % 12)
    }

    /// Parse a class name, STRICTLY: one letter A–G (either case), then an
    /// optional single accidental — `#` or `s` for sharp, `b` (ASCII) for
    /// flat. Enharmonics are valid (`"Cb"` = B, `"B#"` = C); anything else —
    /// `"H"`, `"C##"`, `"Fis"`, the empty string — is an error naming the
    /// grammar.
    pub fn from_name(name: &str) -> Result<PitchClass, MusicError> {
        let expected =
            r#"one letter A–G with an optional #/s/b accidental, like "C", "F#", or "Gb""#;
        let bytes = name.as_bytes();
        if bytes.is_empty() || bytes.len() > 2 {
            return Err(bad_name("pitch-class", name, expected));
        }
        let Some(mut semis) = letter_semitones(bytes[0]) else {
            return Err(bad_name("pitch-class", name, expected));
        };
        match bytes.get(1) {
            None => {}
            Some(b'#') | Some(b's') => semis += 1,
            Some(b'b') => semis -= 1,
            _ => return Err(bad_name("pitch-class", name, expected)),
        }
        // rem_euclid keeps the enharmonic edges in range: Cb (-1) → 11 = B,
        // B# (12) → 0 = C.
        Ok(PitchClass(semis.rem_euclid(12) as u8))
    }

    /// The semitone number, 0..=11 (C = 0).
    pub const fn semitone(&self) -> u8 {
        self.0
    }

    /// The sharp spelling (`"C"`, `"F#"`).
    pub fn sharp_name(&self) -> &'static str {
        [
            "C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B",
        ][self.0 as usize]
    }

    /// The flat spelling (`"C"`, `"Gb"`).
    pub fn flat_name(&self) -> &'static str {
        [
            "C", "Db", "D", "Eb", "E", "F", "Gb", "G", "Ab", "A", "Bb", "B",
        ][self.0 as usize]
    }
}

impl std::fmt::Display for PitchClass {
    /// The sharp spelling (`"F#"`) — canonical for the whole module.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.sharp_name())
    }
}

impl Serialize for PitchClass {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_u8(self.0)
    }
}

impl<'de> Deserialize<'de> for PitchClass {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let semitone = u8::deserialize(deserializer)?;
        if semitone > 11 {
            return Err(serde::de::Error::custom(format!(
                "pitch-class {semitone} is out of range — pitch classes span 0..=11 (C..B)"
            )));
        }
        Ok(PitchClass(semitone))
    }
}

/// A distance between two pitches, in semitones (an octave is 12). Signed:
/// negative intervals descend.
///
/// The representation is `i16`, not `i8`: the full MIDI range already spans
/// 127 semitones, which *just* fits an `i8` but leaves zero headroom — every
/// compound interval (a ninth is 14, two octaves 24) and every intermediate
/// in transposition arithmetic would live one step from overflow. `i16`
/// covers anything musically nameable with room to spare while staying
/// exact.
///
/// Serde is the bare semitone count (`7`, `-3`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Interval(i16);

impl Interval {
    /// No distance — the same pitch (`"P1"`).
    pub const UNISON: Interval = Interval(0);
    /// One semitone (`"m2"`).
    pub const MINOR_SECOND: Interval = Interval(1);
    /// Two semitones (`"M2"`).
    pub const MAJOR_SECOND: Interval = Interval(2);
    /// Three semitones (`"m3"`).
    pub const MINOR_THIRD: Interval = Interval(3);
    /// Four semitones (`"M3"`).
    pub const MAJOR_THIRD: Interval = Interval(4);
    /// Five semitones (`"P4"`).
    pub const PERFECT_FOURTH: Interval = Interval(5);
    /// Six semitones — the augmented fourth / diminished fifth (`"TT"`).
    pub const TRITONE: Interval = Interval(6);
    /// Seven semitones (`"P5"`).
    pub const PERFECT_FIFTH: Interval = Interval(7);
    /// Eight semitones (`"m6"`).
    pub const MINOR_SIXTH: Interval = Interval(8);
    /// Nine semitones (`"M6"`).
    pub const MAJOR_SIXTH: Interval = Interval(9);
    /// Ten semitones (`"m7"`).
    pub const MINOR_SEVENTH: Interval = Interval(10);
    /// Eleven semitones (`"M7"`).
    pub const MAJOR_SEVENTH: Interval = Interval(11);
    /// Twelve semitones — the same class, one register up (`"P8"`).
    pub const OCTAVE: Interval = Interval(12);

    /// An interval of `semitones` (negative descends).
    pub const fn new(semitones: i16) -> Interval {
        Interval(semitones)
    }

    /// The distance in semitones.
    pub const fn semitones(&self) -> i16 {
        self.0
    }
}

impl std::fmt::Display for Interval {
    /// The standard quality within an octave (`"m3"`, `"P5"`, `"TT"` for the
    /// tritone); a signed semitone count outside (`"+14"`, `"-3"` — negative
    /// intervals never get quality names).
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let name = match self.0 {
            0 => "P1",
            1 => "m2",
            2 => "M2",
            3 => "m3",
            4 => "M3",
            5 => "P4",
            6 => "TT",
            7 => "P5",
            8 => "m6",
            9 => "M6",
            10 => "m7",
            11 => "M7",
            12 => "P8",
            n => return write!(f, "{n:+}"),
        };
        f.write_str(name)
    }
}

/// An absolute pitch: a MIDI note number, 0..=127 (`"C-1"` to `"G9"`; middle
/// C, `"C4"`, is 60 and `"A4"` is 69). Ordering and equality are by MIDI
/// number, so enharmonic spellings compare equal.
///
/// Serde is the name string (`"F#3"`) — deserialized through the SAME strict
/// grammar as [`Pitch::from_name`], so an invalid name fails deserialization
/// instead of being silently guessed at.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Pitch(u8);

impl Pitch {
    /// The pitch with MIDI number `midi` (0..=127 — the upper half of `u8`
    /// is an error, not a wrap).
    pub fn from_midi(midi: u8) -> Result<Pitch, MusicError> {
        Self::from_midi_i32(midi as i32)
    }

    /// The pitch of `class` in `octave` (scientific pitch notation: octave 4
    /// contains middle C). `midi = (octave + 1) × 12 + class`; anything
    /// outside 0..=127 is an error — never a clamp.
    pub fn new(class: PitchClass, octave: i8) -> Result<Pitch, MusicError> {
        Self::from_midi_i32((octave as i32 + 1) * 12 + class.0 as i32)
    }

    /// Parse a pitch name, STRICTLY: a [`PitchClass`] name plus a signed
    /// octave (`"C4"`, `"F#3"`, `"Gb5"`, `"F#-1"`), or `"midi:N"` with N an
    /// integer (`"midi:60"` = C4), for parity with the DSL. This is a strict
    /// subset of the DSL's `Value::Note` spellings — same convention, no
    /// leniency: the octave is mandatory and accidentals are single.
    pub fn from_name(name: &str) -> Result<Pitch, MusicError> {
        let expected = r#"a note name like "C4", "F#3", "Gb5", or "midi:60""#;
        if let Some(num) = name.strip_prefix("midi:") {
            let midi: i16 = num.parse().map_err(|_| bad_name("pitch", name, expected))?;
            return Self::from_midi_i32(midi as i32);
        }
        let bytes = name.as_bytes();
        let Some((&letter, mut rest)) = bytes.split_first() else {
            return Err(bad_name("pitch", name, expected));
        };
        let Some(mut semis) = letter_semitones(letter) else {
            return Err(bad_name("pitch", name, expected));
        };
        // One optional accidental; everything after it must be the octave.
        if let Some((&acc, tail)) = rest.split_first() {
            match acc {
                b'#' | b's' => {
                    semis += 1;
                    rest = tail;
                }
                b'b' => {
                    semis -= 1;
                    rest = tail;
                }
                _ => {}
            }
        }
        let octave: i8 = std::str::from_utf8(rest)
            .ok()
            .filter(|s| !s.is_empty())
            .and_then(|s| s.parse().ok())
            .ok_or_else(|| bad_name("pitch", name, expected))?;
        // Enharmonic edges (Cb4, B#3) land on the neighbouring class through
        // the same midi formula, so no special case is needed.
        Self::from_midi_i32((octave as i32 + 1) * 12 + semis as i32)
    }

    /// The MIDI note number, 0..=127.
    pub const fn to_midi(&self) -> u8 {
        self.0
    }

    /// The pitch class (the note name without its octave).
    pub const fn pitch_class(&self) -> PitchClass {
        PitchClass(self.0 % 12)
    }

    /// The octave in scientific pitch notation (`"C4"` → 4, `"B-1"` → −1).
    pub const fn octave(&self) -> i8 {
        (self.0 / 12) as i8 - 1
    }

    /// The frequency in Hz: 12-tone equal temperament with A4 = 440 — the
    /// SAME formula the DSL's [`note_to_hz`](crate::dsl::note_to_hz) applies
    /// to `Value::Note` strings, `440 × 2^((midi − 69) / 12)`, so a pitch and
    /// its DSL spelling render at identical frequencies. This is the one
    /// place the module leaves integer arithmetic.
    pub fn to_hz(&self) -> f32 {
        440.0 * 2f32.powf((self.0 as f32 - 69.0) / 12.0)
    }

    /// The pitch `interval` away, erroring if that lands outside 0..=127.
    pub fn transpose(&self, interval: Interval) -> Result<Pitch, MusicError> {
        self.add_semitones(interval.semitones())
    }

    /// The pitch `semitones` away (negative descends), erroring outside
    /// 0..=127 — transposition never wraps or clamps.
    pub fn add_semitones(&self, semitones: i16) -> Result<Pitch, MusicError> {
        Self::from_midi_i32(self.0 as i32 + semitones as i32)
    }

    /// The pitch of MIDI number `midi`, or an out-of-range error naming the
    /// span. Shared by every constructor so the bounds live in exactly one
    /// place.
    fn from_midi_i32(midi: i32) -> Result<Pitch, MusicError> {
        if (0..=127).contains(&midi) {
            Ok(Pitch(midi as u8))
        } else {
            Err(MusicError::OutOfRange(format!(
                "pitch midi {midi} is out of range — MIDI pitches span 0..=127 (\"C-1\" to \"G9\")"
            )))
        }
    }
}

impl std::fmt::Display for Pitch {
    /// The sharp spelling plus octave (`"F#3"`, `"C-1"`). Enharmonic input
    /// (`"Gb3"`) displays canonically (`"F#3"`).
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}{}", self.pitch_class().sharp_name(), self.octave())
    }
}

impl Serialize for Pitch {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for Pitch {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let name = String::deserialize(deserializer)?;
        Pitch::from_name(&name).map_err(serde::de::Error::custom)
    }
}

/// A named interval pattern: the semitone steps from a tonic that define a
/// scale. Carries no tonic itself — pair one with a root in a [`Key`].
///
/// Serde is the canonical lowercase name (`"major"`, `"natural_minor"`, …) —
/// the same strings [`Scale::from_name`] accepts (plus its `"minor"` short
/// form).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Scale {
    /// The major (Ionian) scale.
    Major,
    /// The natural minor (Aeolian) scale.
    NaturalMinor,
    /// Natural minor with a raised 7th — the leading tone, for harmony.
    HarmonicMinor,
    /// Natural minor with raised 6th and 7th (the ascending form).
    MelodicMinor,
    /// The five-note major pentatonic (1 2 3 5 6).
    MajorPentatonic,
    /// The five-note minor pentatonic (1 ♭3 4 5 ♭7).
    MinorPentatonic,
    /// The Dorian mode: minor with a raised 6th.
    Dorian,
    /// The Mixolydian mode: major with a lowered 7th.
    Mixolydian,
    /// All twelve pitch classes.
    Chromatic,
}

impl Scale {
    /// The semitone steps from the tonic, ascending — the first is always 0
    /// (the tonic itself).
    pub fn intervals(&self) -> &'static [u8] {
        match self {
            Scale::Major => &[0, 2, 4, 5, 7, 9, 11],
            Scale::NaturalMinor => &[0, 2, 3, 5, 7, 8, 10],
            Scale::HarmonicMinor => &[0, 2, 3, 5, 7, 8, 11],
            Scale::MelodicMinor => &[0, 2, 3, 5, 7, 9, 11],
            Scale::MajorPentatonic => &[0, 2, 4, 7, 9],
            Scale::MinorPentatonic => &[0, 3, 5, 7, 10],
            Scale::Dorian => &[0, 2, 3, 5, 7, 9, 10],
            Scale::Mixolydian => &[0, 2, 4, 5, 7, 9, 10],
            Scale::Chromatic => &[0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11],
        }
    }

    /// The canonical lowercase name — the serde spelling, and what
    /// [`Scale::from_name`] round-trips through.
    pub fn name(&self) -> &'static str {
        match self {
            Scale::Major => "major",
            Scale::NaturalMinor => "natural_minor",
            Scale::HarmonicMinor => "harmonic_minor",
            Scale::MelodicMinor => "melodic_minor",
            Scale::MajorPentatonic => "major_pentatonic",
            Scale::MinorPentatonic => "minor_pentatonic",
            Scale::Dorian => "dorian",
            Scale::Mixolydian => "mixolydian",
            Scale::Chromatic => "chromatic",
        }
    }

    /// Parse a scale name, STRICTLY lowercase: the canonical names
    /// (`"major"`, `"natural_minor"`, `"harmonic_minor"`, `"melodic_minor"`,
    /// `"major_pentatonic"`, `"minor_pentatonic"`, `"dorian"`,
    /// `"mixolydian"`, `"chromatic"`) plus the short form `"minor"` for
    /// natural minor (so `"A minor"` reads naturally as a key). Anything
    /// else — `"Major"`, `"ionian"`, the empty string — is an error naming
    /// the list.
    pub fn from_name(name: &str) -> Result<Scale, MusicError> {
        match name {
            "major" => Ok(Scale::Major),
            "minor" | "natural_minor" => Ok(Scale::NaturalMinor),
            "harmonic_minor" => Ok(Scale::HarmonicMinor),
            "melodic_minor" => Ok(Scale::MelodicMinor),
            "major_pentatonic" => Ok(Scale::MajorPentatonic),
            "minor_pentatonic" => Ok(Scale::MinorPentatonic),
            "dorian" => Ok(Scale::Dorian),
            "mixolydian" => Ok(Scale::Mixolydian),
            "chromatic" => Ok(Scale::Chromatic),
            _ => Err(bad_name(
                "scale",
                name,
                r#"one of "major", "minor", "natural_minor", "harmonic_minor", "melodic_minor", "major_pentatonic", "minor_pentatonic", "dorian", "mixolydian", or "chromatic""#,
            )),
        }
    }

    /// Is `class` one of the pattern's steps? Membership is keyed to C —
    /// `Scale::Major.contains(G)` is true because G is in C major. For
    /// membership in an actual key, use [`Key::contains`].
    pub fn contains(&self, class: PitchClass) -> bool {
        self.intervals().contains(&class.semitone())
    }

    /// The interval of scale degree `n` above the tonic, 1-based: degree 1
    /// is unison, degree 2 a step up, and degrees past the scale length wrap
    /// by octaves (degree 8 of a seven-note scale is the octave). Degree 0
    /// is an error — there is no zeroth degree.
    pub fn degree(&self, n: u32) -> Result<Interval, MusicError> {
        if n == 0 {
            return Err(MusicError::BadDegree(0));
        }
        let steps = self.intervals();
        let index = (n - 1) as usize;
        let octaves = (index / steps.len()) as i64 * 12;
        let semitones = steps[index % steps.len()] as i64 + octaves;
        let semitones = i16::try_from(semitones).map_err(|_| {
            MusicError::OutOfRange(format!("scale degree {n} is too large for an interval"))
        })?;
        Ok(Interval::new(semitones))
    }

    /// The pitch classes of the scale built on `tonic`, ascending from the
    /// tonic (`Scale::Major.notes(C)` = C D E F G A B).
    pub fn notes(&self, tonic: PitchClass) -> Vec<PitchClass> {
        self.intervals()
            .iter()
            .map(|&step| PitchClass::from_semitone(tonic.semitone() + step))
            .collect()
    }
}

impl std::fmt::Display for Scale {
    /// The canonical lowercase name (`"natural_minor"`).
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.name())
    }
}

/// A key: a [`Scale`] rooted at a tonic [`PitchClass`] — the thing "C major"
/// names. Answers the two everyday questions: which pitches belong
/// ([`Key::contains`]) and what a degree resolves to ([`Key::degree_pitch`]).
///
/// Serde is the explicit struct `{"tonic": 0, "scale": "major"}` (tonic as
/// the semitone 0..=11, scale as its canonical lowercase name) — the
/// explicit-field style of the composition layer, NOT the display string.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Key {
    /// The pitch class the scale is built on.
    pub tonic: PitchClass,
    /// The interval pattern.
    pub scale: Scale,
}

impl Key {
    /// The key of `scale` built on `tonic`.
    pub const fn new(tonic: PitchClass, scale: Scale) -> Key {
        Key { tonic, scale }
    }

    /// Parse a key name, STRICTLY: a [`PitchClass`] name, exactly one space,
    /// then a [`Scale`] name — `"C major"`, `"A minor"`, `"F# dorian"`.
    /// Anything else (`"Cmajor"`, `"C  major"`, `"C ionian"`) is an error.
    pub fn from_name(name: &str) -> Result<Key, MusicError> {
        let expected =
            r#"a tonic, one space, and a scale name, like "C major", "A minor", or "F# dorian""#;
        let Some((tonic, scale)) = name.split_once(' ') else {
            return Err(bad_name("key", name, expected));
        };
        let tonic = PitchClass::from_name(tonic).map_err(|_| bad_name("key", name, expected))?;
        let scale = Scale::from_name(scale).map_err(|_| bad_name("key", name, expected))?;
        Ok(Key { tonic, scale })
    }

    /// The pitch of scale degree `n` (1-based) in `octave` — degree 3 of C
    /// major at octave 4 is E4, and degrees past the scale length cross into
    /// the next octave (degree 8 is the tonic an octave up). Errors on
    /// degree 0 or a result outside the MIDI range.
    pub fn degree_pitch(&self, n: u32, octave: i8) -> Result<Pitch, MusicError> {
        Pitch::new(self.tonic, octave)?.transpose(self.scale.degree(n)?)
    }

    /// Does `pitch` belong to the key?
    pub fn contains(&self, pitch: Pitch) -> bool {
        self.scale.notes(self.tonic).contains(&pitch.pitch_class())
    }
}

impl std::fmt::Display for Key {
    /// The sharp tonic plus the canonical scale name (`"C major"`,
    /// `"A natural_minor"`). Parses back to the same key via
    /// [`Key::from_name`].
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} {}", self.tonic.sharp_name(), self.scale.name())
    }
}

/// The quality of a [`Chord`]: which intervals sit above the root.
///
/// Serde is the canonical snake_case name (`"major"`,
/// `"dominant_seventh"`, …).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChordQuality {
    /// Root, major third, perfect fifth (`"C"`).
    Major,
    /// Root, minor third, perfect fifth (`"Cm"`).
    Minor,
    /// Root, minor third, diminished fifth (`"Cdim"`).
    Diminished,
    /// Root, major third, augmented fifth (`"Caug"`).
    Augmented,
    /// A major triad plus a major seventh (`"Cmaj7"`).
    MajorSeventh,
    /// A minor triad plus a minor seventh (`"Cm7"`).
    MinorSeventh,
    /// A major triad plus a minor seventh (`"C7"`) — the dominant function.
    DominantSeventh,
}

impl ChordQuality {
    /// The semitone steps above the root, ascending from 0.
    pub fn intervals(&self) -> &'static [u8] {
        match self {
            ChordQuality::Major => &[0, 4, 7],
            ChordQuality::Minor => &[0, 3, 7],
            ChordQuality::Diminished => &[0, 3, 6],
            ChordQuality::Augmented => &[0, 4, 8],
            ChordQuality::MajorSeventh => &[0, 4, 7, 11],
            ChordQuality::MinorSeventh => &[0, 3, 7, 10],
            ChordQuality::DominantSeventh => &[0, 4, 7, 10],
        }
    }

    /// The strict-grammar spelling that follows the root in a chord name
    /// (`""`, `"m"`, `"maj7"`, `"m7"`, `"7"`, `"dim"`, `"aug"`).
    pub fn suffix(&self) -> &'static str {
        match self {
            ChordQuality::Major => "",
            ChordQuality::Minor => "m",
            ChordQuality::Diminished => "dim",
            ChordQuality::Augmented => "aug",
            ChordQuality::MajorSeventh => "maj7",
            ChordQuality::MinorSeventh => "m7",
            ChordQuality::DominantSeventh => "7",
        }
    }
}

/// A chord: a root [`PitchClass`] plus a [`ChordQuality`].
///
/// Serde is the explicit struct `{"root": 0, "quality": "major"}` (root as
/// the semitone 0..=11, quality as its canonical name) — explicit like
/// [`Key`], not the display string.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Chord {
    /// The root.
    pub root: PitchClass,
    /// The quality.
    pub quality: ChordQuality,
}

impl Chord {
    /// The chord of `quality` built on `root`.
    pub const fn new(root: PitchClass, quality: ChordQuality) -> Chord {
        Chord { root, quality }
    }

    /// Parse a chord name, STRICTLY: a [`PitchClass`] root followed by one
    /// of exactly seven suffixes — `""` (major), `"m"`, `"maj7"`, `"m7"`,
    /// `"7"`, `"dim"`, `"aug"`. Anything else — `"CM"`, `"Cmaj"`, `"Csus4"`,
    /// `"C sus4"` — is an error naming the grammar; a chord spelling is
    /// never guessed.
    pub fn from_name(name: &str) -> Result<Chord, MusicError> {
        let expected = r#"a root plus one of "", "m", "maj7", "m7", "7", "dim", or "aug", like "C", "Cm", or "Cmaj7""#;
        if !name.is_ascii() || name.is_empty() {
            return Err(bad_name("chord", name, expected));
        }
        let bytes = name.as_bytes();
        // The root is the letter plus at most one accidental — 1 or 2 bytes.
        // No valid suffix starts with an accidental character, so this split
        // is unambiguous.
        let root_len = match bytes.get(1) {
            Some(b'#') | Some(b's') | Some(b'b') => 2,
            _ => 1,
        };
        let (root, suffix) = name.split_at(root_len);
        let root = PitchClass::from_name(root).map_err(|_| bad_name("chord", name, expected))?;
        let quality = match suffix {
            "" => ChordQuality::Major,
            "m" => ChordQuality::Minor,
            "maj7" => ChordQuality::MajorSeventh,
            "m7" => ChordQuality::MinorSeventh,
            "7" => ChordQuality::DominantSeventh,
            "dim" => ChordQuality::Diminished,
            "aug" => ChordQuality::Augmented,
            _ => return Err(bad_name("chord", name, expected)),
        };
        Ok(Chord { root, quality })
    }

    /// The pitch classes of the chord, ascending from the root — C major is
    /// `[C, E, G]`. The root is always first; this is a set, not a spacing
    /// (see [`Voicing`] for that).
    pub fn notes(&self) -> Vec<PitchClass> {
        self.quality
            .intervals()
            .iter()
            .map(|&step| PitchClass::from_semitone(self.root.semitone() + step))
            .collect()
    }

    /// The `n`-th inversion: the lowest `n` voices raised an octave, so the
    /// `n`-th chord tone becomes the bass (`C.invert(1)` is C/E, `[E, G,
    /// C]`). `n` wraps modulo the chord size, so `invert(3)` of a triad is
    /// root position again.
    pub fn invert(&self, n: u32) -> Inversion {
        Inversion {
            chord: *self,
            inversion: n,
        }
    }

    /// The ascending arpeggio from the root in `octave` — the same pitches
    /// as a root-position close [`Voicing`]. Errors if a note lands outside
    /// the MIDI range (a high root plus a wide quality runs past G9).
    pub fn arp(&self, octave: i8) -> Result<Vec<Pitch>, MusicError> {
        let root = Pitch::new(self.root, octave)?;
        self.quality
            .intervals()
            .iter()
            .map(|&step| root.add_semitones(step as i16))
            .collect()
    }

    /// Is `class` one of the chord tones?
    pub fn contains(&self, class: PitchClass) -> bool {
        self.notes().contains(&class)
    }
}

impl std::fmt::Display for Chord {
    /// The sharp root plus the quality suffix (`"Cm"`, `"F#maj7"`). Parses
    /// back to the same chord via [`Chord::from_name`].
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}{}", self.root.sharp_name(), self.quality.suffix())
    }
}

/// A chord in inversion: which chord tone sits in the bass. Made by
/// [`Chord::invert`]; re-voices the chord's pitch classes with the former
/// bass voices raised an octave.
///
/// Serde is the explicit struct `{"chord": {...}, "inversion": 1}`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Inversion {
    /// The chord being inverted.
    pub chord: Chord,
    /// The inversion number, taken modulo the chord size when the voicing is
    /// computed (0 = root position; 1 puts a triad's third in the bass).
    pub inversion: u32,
}

impl Inversion {
    /// The re-voiced pitch classes, bass first: the classes of
    /// [`Chord::notes`] rotated so the `n`-th chord tone leads
    /// (`C.invert(1)` → `[E, G, C]`).
    pub fn pitch_classes(&self) -> Vec<PitchClass> {
        let notes = self.chord.notes();
        let n = (self.inversion as usize) % notes.len();
        notes[n..].iter().chain(&notes[..n]).copied().collect()
    }

    /// The inversion realized as ascending pitches from `octave`: the voices
    /// rotated below the bass are raised an octave, so the sequence climbs
    /// (C/E at octave 4 is E4 G4 C5). Errors outside the MIDI range.
    pub fn pitches(&self, octave: i8) -> Result<Vec<Pitch>, MusicError> {
        let root = Pitch::new(self.chord.root, octave)?;
        let intervals = self.chord.quality.intervals();
        let n = (self.inversion as usize) % intervals.len();
        intervals[n..]
            .iter()
            .map(|&step| root.add_semitones(step as i16))
            .chain(
                intervals[..n]
                    .iter()
                    .map(|&step| root.add_semitones(step as i16 + 12)),
            )
            .collect()
    }
}

/// A concrete spacing of a chord: actual pitches, low to high. Built by the
/// constructors below (close position, drop-2 open, slash bass) or
/// transposed from another voicing — every constructor keeps the voices
/// strictly ascending and INSIDE the MIDI range, erroring rather than
/// clamping when a spacing wouldn't fit.
///
/// Serde is the explicit struct `{"pitches": ["E3", "C4", "G4"]}`;
/// deserializing re-checks the invariant (non-empty, strictly ascending).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
pub struct Voicing {
    pitches: Vec<Pitch>,
}

impl Voicing {
    /// Root-position close voicing: the chord tones stacked within an
    /// octave, root in the bass (C at octave 4 = C4 E4 G4).
    pub fn close(chord: Chord, octave: i8) -> Result<Voicing, MusicError> {
        Voicing::checked(chord.arp(octave)?)
    }

    /// Drop-2 open voicing: the close voicing with the SECOND-highest voice
    /// dropped an octave (C at octave 4 = E3 C4 G4). The dropped voice needs
    /// room below — a close voicing starting near the bottom of the MIDI
    /// range errors instead of wrapping.
    pub fn open(chord: Chord, octave: i8) -> Result<Voicing, MusicError> {
        let close = chord.arp(octave)?;
        let dropped = close[close.len() - 2].add_semitones(-12)?;
        let mut pitches = vec![dropped];
        pitches.extend_from_slice(&close[..close.len() - 2]);
        pitches.push(close[close.len() - 1]);
        Voicing::checked(pitches)
    }

    /// A slash chord: the close voicing over `bass`, placed in the nearest
    /// octave strictly below the root (C/E at octave 4 = E3 C4 E4 G4).
    /// Errors when there is no room for the bass below the root.
    pub fn with_bass(chord: Chord, octave: i8, bass: PitchClass) -> Result<Voicing, MusicError> {
        let close = chord.arp(octave)?;
        let root = close[0].to_midi() as i16;
        let mut bass_midi = (octave as i16 + 1) * 12 + bass.semitone() as i16;
        if bass_midi >= root {
            bass_midi -= 12;
        }
        let bass = Pitch::from_midi_i32(bass_midi as i32)?;
        let mut pitches = vec![bass];
        pitches.extend_from_slice(&close);
        Voicing::checked(pitches)
    }

    /// The voices, strictly ascending (bass first).
    pub fn pitches(&self) -> &[Pitch] {
        &self.pitches
    }

    /// Every voice shifted by `interval`, erroring if any voice would leave
    /// the MIDI range.
    pub fn transpose(&self, interval: Interval) -> Result<Voicing, MusicError> {
        let pitches = self
            .pitches
            .iter()
            .map(|p| p.transpose(interval))
            .collect::<Result<Vec<_>, _>>()?;
        Voicing::checked(pitches)
    }

    /// A voicing from checked pitches — the single chokepoint for the
    /// invariant (non-empty, strictly ascending), shared by the constructors
    /// and serde.
    fn checked(pitches: Vec<Pitch>) -> Result<Voicing, MusicError> {
        if pitches.is_empty() {
            return Err(MusicError::BadVoicing(
                "a voicing needs at least one pitch".to_string(),
            ));
        }
        if pitches.windows(2).any(|pair| pair[0] >= pair[1]) {
            return Err(MusicError::BadVoicing(
                "voicing pitches must be strictly ascending (bass to soprano)".to_string(),
            ));
        }
        Ok(Voicing { pitches })
    }
}

impl<'de> Deserialize<'de> for Voicing {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        struct Raw {
            pitches: Vec<Pitch>,
        }
        Voicing::checked(Raw::deserialize(deserializer)?.pitches).map_err(serde::de::Error::custom)
    }
}

/// Why a music operation failed. Every fallible constructor in the module
/// returns this; the messages are human-readable (they name what was
/// rejected and show an example of the valid grammar) so a tool can
/// pattern-match and self-correct — the same contract as the DSL's
/// validation errors.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MusicError {
    /// A name string didn't match the strict grammar — the message shows
    /// the rejected input and an example of the valid form.
    BadName(String),
    /// A pitch, or a pitch derived from one, landed outside the MIDI range
    /// 0..=127 — the message names the value and the bounds.
    OutOfRange(String),
    /// A scale degree below 1 (degrees are 1-based: 1 = tonic).
    BadDegree(u32),
    /// A voicing violated its structural invariant (empty, or not strictly
    /// ascending).
    BadVoicing(String),
}

impl std::fmt::Display for MusicError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MusicError::BadName(message)
            | MusicError::OutOfRange(message)
            | MusicError::BadVoicing(message) => f.write_str(message),
            MusicError::BadDegree(n) => {
                write!(f, "degree {n} is invalid — degrees are 1-based (1 = tonic)")
            }
        }
    }
}

impl std::error::Error for MusicError {}

/// A `BadName` error with the module's standard message shape: what was
/// rejected, then what would have been accepted.
fn bad_name(kind: &str, input: &str, expected: &str) -> MusicError {
    MusicError::BadName(format!(
        "invalid {kind} name \"{input}\" — expected {expected}"
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pc(name: &str) -> PitchClass {
        PitchClass::from_name(name).unwrap()
    }

    fn pitch(name: &str) -> Pitch {
        Pitch::from_name(name).unwrap()
    }

    fn midis(pitches: &[Pitch]) -> Vec<u8> {
        pitches.iter().map(|p| p.to_midi()).collect()
    }

    #[test]
    fn pitch_class_from_semitone_wraps() {
        assert_eq!(PitchClass::from_semitone(0).semitone(), 0);
        assert_eq!(PitchClass::from_semitone(11).semitone(), 11);
        assert_eq!(PitchClass::from_semitone(12).semitone(), 0);
        assert_eq!(PitchClass::from_semitone(13).semitone(), 1);
        assert_eq!(PitchClass::from_semitone(255).semitone(), 3);
    }

    #[test]
    fn pitch_class_names_parse_strictly() {
        assert_eq!(pc("C").semitone(), 0);
        assert_eq!(pc("F#").semitone(), 6);
        assert_eq!(pc("Gb").semitone(), 6);
        assert_eq!(pc("f#").semitone(), 6); // the letter is case-insensitive
        assert_eq!(pc("Fs").semitone(), 6); // s = sharp
        assert_eq!(pc("Cb").semitone(), 11); // enharmonic: Cb = B
        assert_eq!(pc("B#").semitone(), 0); // enharmonic: B# = C
        assert_eq!(pc("E#").semitone(), 5);
        assert_eq!(pc("Fb").semitone(), 4);
        for bad in ["", "H", "C##", "Fis", "Cb#", "C b", "CC"] {
            assert!(
                PitchClass::from_name(bad).is_err(),
                "{bad:?} must not parse"
            );
        }
    }

    #[test]
    fn pitch_class_displays_with_sharps() {
        assert_eq!(pc("Gb").to_string(), "F#");
        assert_eq!(pc("Bb").to_string(), "A#");
        assert_eq!(pc("C").to_string(), "C");
        assert_eq!(pc("Gb").flat_name(), "Gb");
        assert_eq!(pc("E").flat_name(), "E"); // naturals spell the same
    }

    #[test]
    fn pitch_names_round_trip() {
        for name in ["C4", "F#3", "A4", "C-1", "G9", "Bb2"] {
            let p = pitch(name);
            assert_eq!(pitch(&p.to_string()), p, "{name} must round-trip");
        }
        // Enharmonics: same midi, canonical (sharp) Display.
        assert_eq!(pitch("C#4"), pitch("Db4"));
        assert_eq!(pitch("Db4").to_string(), "C#4");
        assert_eq!(pitch("Gb3").to_string(), "F#3");
        // "midi:N" parity with the DSL.
        assert_eq!(pitch("midi:60"), pitch("C4"));
        assert_eq!(pitch("midi:69"), pitch("A4"));
    }

    #[test]
    fn pitch_midi_octave_class_agree() {
        let c4 = pitch("C4");
        assert_eq!(c4.to_midi(), 60);
        assert_eq!(c4.pitch_class(), pc("C"));
        assert_eq!(c4.octave(), 4);
        let b_minus_1 = pitch("B-1");
        assert_eq!(b_minus_1.to_midi(), 11);
        assert_eq!(b_minus_1.octave(), -1);
    }

    #[test]
    fn pitch_new_checks_bounds() {
        assert_eq!(Pitch::new(pc("C"), -1).unwrap().to_midi(), 0);
        assert_eq!(Pitch::new(pc("G"), 9).unwrap().to_midi(), 127);
        assert!(Pitch::new(pc("G#"), 9).is_err()); // 128 — no clamping
        assert!(Pitch::new(pc("C"), -2).is_err());
        assert_eq!(Pitch::from_midi(127).unwrap(), pitch("G9"));
        assert!(Pitch::from_midi(128).is_err());
    }

    #[test]
    fn pitch_grammar_is_strict() {
        for bad in [
            "",
            "H4",
            "C",
            "C##4",
            "C4 ",
            " C4",
            "midi:",
            "midi:60.5",
            "m60",
            "C#b4",
        ] {
            assert!(Pitch::from_name(bad).is_err(), "{bad:?} must not parse");
        }
        assert_eq!(
            Pitch::from_name("C").unwrap_err().to_string(),
            r#"invalid pitch name "C" — expected a note name like "C4", "F#3", "Gb5", or "midi:60""#
        );
        // Out-of-range names fail as OutOfRange, not BadName.
        assert_eq!(
            Pitch::from_name("midi:128").unwrap_err().to_string(),
            r#"pitch midi 128 is out of range — MIDI pitches span 0..=127 ("C-1" to "G9")"#
        );
        assert!(Pitch::from_name("C10").is_err());
        assert!(Pitch::from_name("G#9").is_err());
    }

    #[test]
    fn pitch_to_hz_matches_the_dsl_formula() {
        // A4 = 440 exactly, and every pitch agrees with the DSL's note_to_hz
        // on the same spelling.
        assert_eq!(pitch("A4").to_hz(), 440.0);
        for name in ["C4", "F#3", "Gb5", "C-1", "G9"] {
            assert_eq!(
                pitch(name).to_hz(),
                crate::dsl::note_to_hz(name).unwrap(),
                "{name} must match note_to_hz"
            );
        }
        let c4 = pitch("C4").to_hz();
        assert!((c4 - 261.63).abs() < 0.01, "C4 ≈ 261.63 Hz, got {c4}");
    }

    #[test]
    fn interval_constants_and_display() {
        assert_eq!(Interval::UNISON.semitones(), 0);
        assert_eq!(Interval::MINOR_THIRD.semitones(), 3);
        assert_eq!(Interval::MAJOR_THIRD.semitones(), 4);
        assert_eq!(Interval::PERFECT_FIFTH.semitones(), 7);
        assert_eq!(Interval::TRITONE.semitones(), 6);
        assert_eq!(Interval::MINOR_SEVENTH.semitones(), 10);
        assert_eq!(Interval::MAJOR_SEVENTH.semitones(), 11);
        assert_eq!(Interval::OCTAVE.semitones(), 12);
        // Display names the qualities within an octave…
        assert_eq!(Interval::UNISON.to_string(), "P1");
        assert_eq!(Interval::MINOR_THIRD.to_string(), "m3");
        assert_eq!(Interval::MAJOR_THIRD.to_string(), "M3");
        assert_eq!(Interval::PERFECT_FIFTH.to_string(), "P5");
        assert_eq!(Interval::TRITONE.to_string(), "TT");
        assert_eq!(Interval::OCTAVE.to_string(), "P8");
        // …and falls back to signed semitones outside.
        assert_eq!(Interval::new(14).to_string(), "+14");
        assert_eq!(Interval::new(-3).to_string(), "-3");
    }

    #[test]
    fn transpose_crosses_octaves_and_checks_bounds() {
        assert_eq!(
            pitch("C4").transpose(Interval::MAJOR_THIRD).unwrap(),
            pitch("E4")
        );
        // Across the octave boundary.
        assert_eq!(
            pitch("B4").transpose(Interval::MINOR_SECOND).unwrap(),
            pitch("C5")
        );
        assert_eq!(pitch("C4").add_semitones(-12).unwrap(), pitch("C3"));
        // The top and bottom of the range are hard errors, never wraps.
        assert!(pitch("G9").transpose(Interval::MINOR_SECOND).is_err());
        assert!(pitch("C-1").add_semitones(-1).is_err());
        assert!(pitch("G9").add_semitones(i16::MAX).is_err());
        assert!(pitch("C-1").add_semitones(i16::MIN).is_err());
    }

    #[test]
    fn pitches_order_by_midi() {
        assert!(pitch("C4") < pitch("C5"));
        assert!(pitch("F#3") == pitch("Gb3")); // enharmonics compare equal
        let mut v = vec![pitch("E4"), pitch("C4"), pitch("G4")];
        v.sort();
        assert_eq!(v, vec![pitch("C4"), pitch("E4"), pitch("G4")]);
    }

    #[test]
    fn scale_intervals_are_the_named_patterns() {
        assert_eq!(Scale::Major.intervals(), &[0, 2, 4, 5, 7, 9, 11]);
        assert_eq!(Scale::NaturalMinor.intervals(), &[0, 2, 3, 5, 7, 8, 10]);
        assert_eq!(Scale::HarmonicMinor.intervals(), &[0, 2, 3, 5, 7, 8, 11]);
        assert_eq!(Scale::MelodicMinor.intervals(), &[0, 2, 3, 5, 7, 9, 11]);
        assert_eq!(Scale::MajorPentatonic.intervals(), &[0, 2, 4, 7, 9]);
        assert_eq!(Scale::MinorPentatonic.intervals(), &[0, 3, 5, 7, 10]);
        assert_eq!(Scale::Dorian.intervals(), &[0, 2, 3, 5, 7, 9, 10]);
        assert_eq!(Scale::Mixolydian.intervals(), &[0, 2, 4, 5, 7, 9, 10]);
        assert_eq!(
            Scale::Chromatic.intervals(),
            &[0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11]
        );
    }

    #[test]
    fn scale_notes_build_on_a_tonic() {
        let semis = |scale: Scale, tonic: &str| {
            scale
                .notes(pc(tonic))
                .iter()
                .map(|p| p.semitone())
                .collect::<Vec<_>>()
        };
        assert_eq!(semis(Scale::Major, "C"), vec![0, 2, 4, 5, 7, 9, 11]); // C D E F G A B
        // A natural minor: the same classes as C major, A first.
        assert_eq!(semis(Scale::NaturalMinor, "A"), vec![9, 11, 0, 2, 4, 5, 7]);
        assert_eq!(semis(Scale::Dorian, "D"), vec![2, 4, 5, 7, 9, 11, 0]);
        assert_eq!(semis(Scale::MinorPentatonic, "A"), vec![9, 0, 2, 4, 7]);
    }

    #[test]
    fn scale_degrees_are_one_based() {
        assert_eq!(Scale::Major.degree(1).unwrap(), Interval::UNISON);
        assert_eq!(Scale::Major.degree(2).unwrap(), Interval::MAJOR_SECOND);
        assert_eq!(Scale::Major.degree(7).unwrap(), Interval::MAJOR_SEVENTH);
        // Degrees past the scale length wrap by octaves.
        assert_eq!(Scale::Major.degree(8).unwrap(), Interval::OCTAVE);
        assert_eq!(Scale::Major.degree(9).unwrap().semitones(), 14);
        assert_eq!(Scale::MajorPentatonic.degree(6).unwrap(), Interval::OCTAVE);
        assert_eq!(Scale::Major.degree(0), Err(MusicError::BadDegree(0)));
        assert!(Scale::Major.degree(u32::MAX).is_err());
    }

    #[test]
    fn scale_contains_is_pattern_membership() {
        assert!(Scale::Major.contains(pc("E"))); // E is in C major
        assert!(!Scale::Major.contains(pc("C#")));
        assert!(Scale::Chromatic.contains(pc("C#")));
    }

    #[test]
    fn scale_names_are_strict_lowercase() {
        assert_eq!(Scale::from_name("major"), Ok(Scale::Major));
        assert_eq!(Scale::from_name("minor"), Ok(Scale::NaturalMinor));
        assert_eq!(Scale::from_name("natural_minor"), Ok(Scale::NaturalMinor));
        assert_eq!(Scale::from_name("dorian"), Ok(Scale::Dorian));
        for bad in ["", "Major", "ionian", "min", "pentatonic"] {
            assert!(Scale::from_name(bad).is_err(), "{bad:?} must not parse");
        }
        // The canonical names round-trip through from_name.
        for scale in [
            Scale::Major,
            Scale::NaturalMinor,
            Scale::HarmonicMinor,
            Scale::MelodicMinor,
            Scale::MajorPentatonic,
            Scale::MinorPentatonic,
            Scale::Dorian,
            Scale::Mixolydian,
            Scale::Chromatic,
        ] {
            assert_eq!(Scale::from_name(scale.name()), Ok(scale));
        }
    }

    #[test]
    fn key_names_round_trip() {
        let c_major = Key::new(pc("C"), Scale::Major);
        assert_eq!(Key::from_name("C major"), Ok(c_major));
        assert_eq!(
            Key::from_name("A minor"),
            Ok(Key::new(pc("A"), Scale::NaturalMinor))
        );
        assert_eq!(
            Key::from_name("F# dorian"),
            Ok(Key::new(pc("F#"), Scale::Dorian))
        );
        for key in [
            c_major,
            Key::new(pc("A"), Scale::NaturalMinor),
            Key::from_name("F# dorian").unwrap(),
        ] {
            assert_eq!(
                Key::from_name(&key.to_string()),
                Ok(key),
                "{key} must round-trip"
            );
        }
        assert_eq!(
            Key::new(pc("A"), Scale::NaturalMinor).to_string(),
            "A natural_minor"
        );
    }

    #[test]
    fn key_grammar_is_strict() {
        for bad in ["", "Cmajor", "C  major", "C major ", "C ionian", "H major"] {
            assert!(Key::from_name(bad).is_err(), "{bad:?} must not parse");
        }
        assert_eq!(
            Key::from_name("Cmajor").unwrap_err().to_string(),
            r#"invalid key name "Cmajor" — expected a tonic, one space, and a scale name, like "C major", "A minor", or "F# dorian""#
        );
    }

    #[test]
    fn key_degree_pitch_resolves_across_octaves() {
        let c_major = Key::new(pc("C"), Scale::Major);
        assert_eq!(c_major.degree_pitch(1, 4).unwrap(), pitch("C4"));
        assert_eq!(c_major.degree_pitch(3, 4).unwrap(), pitch("E4"));
        assert_eq!(c_major.degree_pitch(7, 4).unwrap(), pitch("B4"));
        // Degree 8 crosses into the next octave.
        assert_eq!(c_major.degree_pitch(8, 4).unwrap(), pitch("C5"));
        // The key's own accidentals apply.
        let g_major = Key::new(pc("G"), Scale::Major);
        assert_eq!(g_major.degree_pitch(7, 4).unwrap(), pitch("F#5"));
        // Degree 0 and out-of-range results error.
        assert!(c_major.degree_pitch(0, 4).is_err());
        assert!(c_major.degree_pitch(8, 9).is_err()); // C10 would be 132
    }

    #[test]
    fn key_contains_checks_membership() {
        let c_major = Key::new(pc("C"), Scale::Major);
        assert!(c_major.contains(pitch("E4")));
        assert!(c_major.contains(pitch("B2")));
        assert!(!c_major.contains(pitch("C#4")));
        assert!(!c_major.contains(pitch("Bb3")));
    }

    #[test]
    fn chord_quality_intervals() {
        assert_eq!(ChordQuality::Major.intervals(), &[0, 4, 7]);
        assert_eq!(ChordQuality::Minor.intervals(), &[0, 3, 7]);
        assert_eq!(ChordQuality::Diminished.intervals(), &[0, 3, 6]);
        assert_eq!(ChordQuality::Augmented.intervals(), &[0, 4, 8]);
        assert_eq!(ChordQuality::MajorSeventh.intervals(), &[0, 4, 7, 11]);
        assert_eq!(ChordQuality::MinorSeventh.intervals(), &[0, 3, 7, 10]);
        assert_eq!(ChordQuality::DominantSeventh.intervals(), &[0, 4, 7, 10]);
    }

    #[test]
    fn chord_names_parse_the_seven_forms() {
        let cases = [
            ("C", ChordQuality::Major),
            ("Cm", ChordQuality::Minor),
            ("Cmaj7", ChordQuality::MajorSeventh),
            ("Cm7", ChordQuality::MinorSeventh),
            ("C7", ChordQuality::DominantSeventh),
            ("Cdim", ChordQuality::Diminished),
            ("Caug", ChordQuality::Augmented),
        ];
        for (name, quality) in cases {
            assert_eq!(Chord::from_name(name), Ok(Chord::new(pc("C"), quality)));
        }
        // Accidentals in the root, either spelling.
        assert_eq!(Chord::from_name("F#m7").unwrap().root, pc("Gb"));
        assert_eq!(Chord::from_name("Bb7").unwrap().root, pc("A#"));
        assert_eq!(Chord::from_name("b7").unwrap().root, pc("B"));
        // Display round-trips through the grammar.
        for name in ["C", "Cm", "Cmaj7", "Cm7", "C7", "Cdim", "Caug", "F#m7"] {
            let chord = Chord::from_name(name).unwrap();
            assert_eq!(Chord::from_name(&chord.to_string()), Ok(chord));
        }
    }

    #[test]
    fn chord_grammar_never_guesses() {
        for bad in [
            "", "CM", "Cmaj", "Csus4", "C sus4", "Cm9", "C7 ", "H", "CM7", "cM",
        ] {
            assert!(Chord::from_name(bad).is_err(), "{bad:?} must not parse");
        }
        assert_eq!(
            Chord::from_name("CM").unwrap_err().to_string(),
            r#"invalid chord name "CM" — expected a root plus one of "", "m", "maj7", "m7", "7", "dim", or "aug", like "C", "Cm", or "Cmaj7""#
        );
    }

    #[test]
    fn chord_notes_and_contains() {
        let c = Chord::from_name("C").unwrap();
        let semis: Vec<u8> = c.notes().iter().map(|p| p.semitone()).collect();
        assert_eq!(semis, vec![0, 4, 7]);
        let d7 = Chord::from_name("D7").unwrap();
        let semis: Vec<u8> = d7.notes().iter().map(|p| p.semitone()).collect();
        assert_eq!(semis, vec![2, 6, 9, 0]); // D F# A C
        assert!(d7.contains(pc("C")));
        assert!(!d7.contains(pc("C#")));
        assert!(c.contains(pc("E")));
    }

    #[test]
    fn chord_inversions_rotate_the_bass_up() {
        let c = Chord::from_name("C").unwrap();
        let semis = |n: u32| {
            c.invert(n)
                .pitch_classes()
                .iter()
                .map(|p| p.semitone())
                .collect::<Vec<_>>()
        };
        assert_eq!(semis(0), vec![0, 4, 7]); // root position
        assert_eq!(semis(1), vec![4, 7, 0]); // C/E
        assert_eq!(semis(2), vec![7, 0, 4]); // C/G
        // n wraps modulo the chord size.
        assert_eq!(semis(3), vec![0, 4, 7]);
        // Realized as pitches, the former bass is raised an octave.
        assert_eq!(
            midis(&c.invert(1).pitches(4).unwrap()),
            vec![64, 67, 72] // E4 G4 C5
        );
    }

    #[test]
    fn chord_arp_ascends_from_the_root() {
        let cmaj7 = Chord::from_name("Cmaj7").unwrap();
        let names: Vec<String> = cmaj7
            .arp(4)
            .unwrap()
            .iter()
            .map(|p| p.to_string())
            .collect();
        assert_eq!(names, ["C4", "E4", "G4", "B4"]);
        // A high root with a wide quality runs past G9 and errors.
        assert!(cmaj7.arp(9).is_err());
        assert!(Chord::from_name("C").unwrap().arp(9).is_ok()); // C9 E9 G9 fits
    }

    #[test]
    fn close_voicing_stacks_in_root_position() {
        let c = Chord::from_name("C").unwrap();
        assert_eq!(
            midis(Voicing::close(c, 4).unwrap().pitches()),
            vec![60, 64, 67] // C4 E4 G4, sorted
        );
        // Out-of-range spacings error instead of clamping.
        assert!(Voicing::close(Chord::from_name("Cmaj7").unwrap(), 9).is_err());
    }

    #[test]
    fn open_voicing_is_drop_two() {
        let c = Chord::from_name("C").unwrap();
        assert_eq!(
            midis(Voicing::open(c, 4).unwrap().pitches()),
            vec![52, 60, 67] // E3 C4 G4 — the third dropped an octave
        );
        let cmaj7 = Chord::from_name("Cmaj7").unwrap();
        assert_eq!(
            midis(Voicing::open(cmaj7, 4).unwrap().pitches()),
            vec![55, 60, 64, 71] // G3 C4 E4 B4
        );
        // No room to drop near the bottom of the range.
        assert!(Voicing::open(c, -1).is_err());
    }

    #[test]
    fn slash_voicing_puts_the_bass_below() {
        let c = Chord::from_name("C").unwrap();
        let bass = |bass: &str| midis(Voicing::with_bass(c, 4, pc(bass)).unwrap().pitches());
        assert_eq!(bass("E"), vec![52, 60, 64, 67]); // C/E: E3 below
        assert_eq!(bass("G"), vec![55, 60, 64, 67]); // C/G
        assert_eq!(bass("C"), vec![48, 60, 64, 67]); // root doubled below
        assert_eq!(bass("B"), vec![59, 60, 64, 67]); // B3 — still below C4
        // No room below a root at the bottom of the range.
        assert!(Voicing::with_bass(c, -1, pc("E")).is_err());
    }

    #[test]
    fn voicing_transpose_checks_every_voice() {
        let c = Chord::from_name("C").unwrap();
        let up = Voicing::close(c, 4)
            .unwrap()
            .transpose(Interval::OCTAVE)
            .unwrap();
        assert_eq!(midis(up.pitches()), vec![72, 76, 79]);
        // G8 in the voicing plus a fourteenth is past G9.
        let high = Voicing::close(c, 8).unwrap(); // C8 E8 G8
        assert!(high.transpose(Interval::new(14)).is_err());
        assert!(
            Voicing::close(c, 0)
                .unwrap()
                .transpose(Interval::new(-13))
                .is_err()
        );
    }

    #[test]
    fn serde_round_trips_every_type() {
        // PitchClass and Interval are bare numbers.
        assert_eq!(serde_json::to_string(&pc("F#")).unwrap(), "6");
        assert_eq!(serde_json::from_str::<PitchClass>("6").unwrap(), pc("F#"));
        assert_eq!(
            serde_json::to_string(&Interval::PERFECT_FIFTH).unwrap(),
            "7"
        );
        assert_eq!(
            serde_json::from_str::<Interval>("-3").unwrap(),
            Interval::new(-3)
        );
        // Enums use their canonical names.
        assert_eq!(
            serde_json::to_string(&Scale::NaturalMinor).unwrap(),
            r#""natural_minor""#
        );
        assert_eq!(
            serde_json::from_str::<Scale>(r#""dorian""#).unwrap(),
            Scale::Dorian
        );
        assert_eq!(
            serde_json::to_string(&ChordQuality::DominantSeventh).unwrap(),
            r#""dominant_seventh""#
        );
        // Pitch is the name string, canonically spelled.
        assert_eq!(serde_json::to_string(&pitch("Gb3")).unwrap(), r#""F#3""#);
        assert_eq!(
            serde_json::from_str::<Pitch>(r#""Gb3""#).unwrap(),
            pitch("F#3")
        );
        // Key and Chord are explicit structs.
        let key = Key::from_name("C major").unwrap();
        assert_eq!(
            serde_json::to_string(&key).unwrap(),
            r#"{"tonic":0,"scale":"major"}"#
        );
        assert_eq!(
            serde_json::from_str::<Key>(r#"{"tonic":0,"scale":"major"}"#).unwrap(),
            key
        );
        let chord = Chord::from_name("C7").unwrap();
        assert_eq!(
            serde_json::to_string(&chord).unwrap(),
            r#"{"root":0,"quality":"dominant_seventh"}"#
        );
        assert_eq!(
            serde_json::from_str::<Chord>(r#"{"root":0,"quality":"dominant_seventh"}"#).unwrap(),
            chord
        );
        // Inversion and Voicing round-trip too.
        let inversion = chord.invert(1);
        let json = serde_json::to_string(&inversion).unwrap();
        assert_eq!(serde_json::from_str::<Inversion>(&json).unwrap(), inversion);
        let voicing = Voicing::with_bass(Chord::from_name("C").unwrap(), 4, pc("E")).unwrap();
        assert_eq!(
            serde_json::to_string(&voicing).unwrap(),
            r#"{"pitches":["E3","C4","E4","G4"]}"#
        );
        assert_eq!(
            serde_json::from_str::<Voicing>(r#"{"pitches":["E3","C4","E4","G4"]}"#).unwrap(),
            voicing
        );
    }

    #[test]
    fn serde_rejects_the_invalid() {
        assert!(serde_json::from_str::<Pitch>(r#""H4""#).is_err());
        assert!(serde_json::from_str::<Pitch>(r#""C""#).is_err());
        // A pitch is the string, never a bare number.
        assert!(serde_json::from_str::<Pitch>("60").is_err());
        // A pitch class above 11 errors instead of wrapping.
        assert!(serde_json::from_str::<PitchClass>("12").is_err());
        assert!(serde_json::from_str::<Scale>(r#""Major""#).is_err());
        // A key is the explicit struct, not the display string.
        assert!(serde_json::from_str::<Key>(r#""C major""#).is_err());
        // The voicing invariant is re-checked on load.
        assert!(serde_json::from_str::<Voicing>(r#"{"pitches":[]}"#).is_err());
        assert!(serde_json::from_str::<Voicing>(r#"{"pitches":["E4","C4"]}"#).is_err());
    }

    #[test]
    fn errors_name_the_rejected_input() {
        assert_eq!(
            PitchClass::from_name("H").unwrap_err().to_string(),
            r#"invalid pitch-class name "H" — expected one letter A–G with an optional #/s/b accidental, like "C", "F#", or "Gb""#
        );
        assert_eq!(
            Scale::Major.degree(0).unwrap_err().to_string(),
            "degree 0 is invalid — degrees are 1-based (1 = tonic)"
        );
        assert_eq!(
            Pitch::new(pc("G#"), 9).unwrap_err().to_string(),
            r#"pitch midi 128 is out of range — MIDI pitches span 0..=127 ("C-1" to "G9")"#
        );
    }
}
