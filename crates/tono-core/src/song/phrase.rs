//! A per-track note writer, handed to the closure of [`Song::add`](super::Song::add).

use crate::dsl::{SeqNote, Value};

/// A moving beat cursor plus placement helpers: absolute (`.at(beat)` then
/// `.note(..)` / `.chord(..)`), sequential (`.play(..)` / `.rest(..)` advance
/// the cursor), and drum hits (`.kick()` / `.snare()` / `.hat()` /
/// `.hit(gm_note)`). Velocity for following notes is set with `.vel(..)`.
pub struct Phrase {
    steps_per_beat: u32,
    cursor_beat: f32,
    velocity: f32,
    pub(super) notes: Vec<SeqNote>,
}

impl Phrase {
    pub(super) fn new(steps_per_beat: u32) -> Self {
        Phrase {
            steps_per_beat: steps_per_beat.max(1),
            cursor_beat: 0.0,
            velocity: 1.0,
            notes: Vec::new(),
        }
    }

    fn step_of(&self, beat: f32) -> u32 {
        (beat.max(0.0) * self.steps_per_beat as f32).round() as u32
    }

    fn len_of(&self, dur_beats: f32) -> u32 {
        ((dur_beats.max(0.0) * self.steps_per_beat as f32).round() as u32).max(1)
    }

    /// Move the cursor to an absolute beat (0 = the song start).
    pub fn at(&mut self, beat: f32) -> &mut Self {
        self.cursor_beat = beat;
        self
    }

    /// Set the velocity (0..1) applied to notes placed after this call.
    pub fn vel(&mut self, velocity: f32) -> &mut Self {
        self.velocity = velocity.clamp(0.0, 1.0);
        self
    }

    /// Place a note at the cursor, `dur_beats` long. Does not move the cursor —
    /// pair with `.at(..)`.
    pub fn note(&mut self, pitch: &str, dur_beats: f32) -> &mut Self {
        self.push(pitch, dur_beats);
        self
    }

    /// Place a chord at the cursor — all pitches at once, `dur_beats` long.
    pub fn chord(&mut self, pitches: &[&str], dur_beats: f32) -> &mut Self {
        for p in pitches {
            self.push(p, dur_beats);
        }
        self
    }

    /// Play a note at the cursor and advance the cursor by `dur_beats` — write a
    /// melody without repeating `.at(..)`.
    pub fn play(&mut self, pitch: &str, dur_beats: f32) -> &mut Self {
        self.push(pitch, dur_beats);
        self.cursor_beat += dur_beats;
        self
    }

    /// Advance the cursor by `dur_beats` without sounding anything (a rest).
    pub fn rest(&mut self, dur_beats: f32) -> &mut Self {
        self.cursor_beat += dur_beats;
        self
    }

    fn push(&mut self, pitch: &str, dur_beats: f32) {
        self.notes.push(SeqNote {
            step: self.step_of(self.cursor_beat),
            len: self.len_of(dur_beats),
            pitch: Value::Note(pitch.to_string()),
            gain: self.velocity,
        });
    }

    /// Hit a drum at the cursor by its General MIDI note (a one-step hit). Only
    /// meaningful on a `Kit` instrument, where the note picks the drum.
    pub fn hit(&mut self, gm_note: u8) -> &mut Self {
        self.notes.push(SeqNote {
            step: self.step_of(self.cursor_beat),
            len: 1,
            pitch: Value::Note(format!("midi:{gm_note}")),
            gain: self.velocity,
        });
        self
    }

    /// Kick drum (GM 36).
    pub fn kick(&mut self) -> &mut Self {
        self.hit(36)
    }
    /// Snare (GM 38).
    pub fn snare(&mut self) -> &mut Self {
        self.hit(38)
    }
    /// Closed hi-hat (GM 42).
    pub fn hat(&mut self) -> &mut Self {
        self.hit(42)
    }
    /// Open hi-hat (GM 46).
    pub fn open_hat(&mut self) -> &mut Self {
        self.hit(46)
    }
    /// Hand clap (GM 39).
    pub fn clap(&mut self) -> &mut Self {
        self.hit(39)
    }
    /// Crash cymbal (GM 49).
    pub fn crash(&mut self) -> &mut Self {
        self.hit(49)
    }
    /// Ride cymbal (GM 51).
    pub fn ride(&mut self) -> &mut Self {
        self.hit(51)
    }
    /// Mid tom (GM 45).
    pub fn tom(&mut self) -> &mut Self {
        self.hit(45)
    }
}
