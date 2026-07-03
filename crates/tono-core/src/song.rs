//! song — compose a full piece by adding instruments and arranging parts.
//!
//! A [`Song`] is the ergonomic layer above the raw graph: you add instrument
//! **tracks**, define reusable **patterns** (phrases on a bar grid), and
//! **arrange** them on a timeline. [`Song::to_doc`] compiles the whole thing
//! down to an ordinary [`SoundDoc`] (a `tracks` root of `seq` tracks), so it
//! renders, mixes, exports, and **replays byte-identically** through the exact
//! same engine as everything else — nothing new in the render path.
//!
//! ```
//! use tono_core::song::{Song, note};
//! use tono_core::dsl::{Adsr, SeqWave};
//!
//! let amp = Adsr { a: 0.005, d: 0.1, s: 0.8, r: 0.2, punch: 0.0 };
//! let mut song = Song::new("groove", 120.0);
//! song.add_track("bass", SeqWave::Bass, amp);
//! song.add_pattern("riff", 1, vec![note(0, 4, "C2"), note(8, 4, "G2")]);
//! song.arrange("bass", "riff", 0);
//! song.arrange("bass", "riff", 1); // same phrase, next bar
//! let doc = song.to_doc().unwrap(); // a normal, deterministic SoundDoc
//! ```

use serde::{Deserialize, Serialize};

use crate::catalog::{Instrument, VoiceParams};
use crate::dsl::{Adsr, ENGINE_VERSION, Node, SeqNote, SeqWave, SoundDoc, Track, Value};

/// One instrument track: an instrument voice plus its mixer settings. Notes come
/// from the patterns arranged onto it.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SongTrack {
    /// Stable track name — patterns are arranged onto it and it becomes the
    /// rendered layer id.
    pub name: String,
    /// The instrument (a synth wave, a built-in instrument like `piano`/`bass`/
    /// `kit`, or `sampler` with a SoundFont).
    pub wave: SeqWave,
    /// The per-note amplitude envelope.
    pub env: Adsr,
    /// Channel fader, 0..2 (1 = unity).
    #[serde(default = "unit_gain")]
    pub gain: f32,
    /// Stereo position, −1 (hard left) .. 1 (hard right).
    #[serde(default)]
    pub pan: f32,
    /// SoundFont path when `wave` is `sampler` (else ignored).
    #[serde(default)]
    pub sf2: String,
    /// General MIDI program when `wave` is `sampler`.
    #[serde(default)]
    pub sf2_preset: u32,
    /// SoundFont bank when `wave` is `sampler` (128 = the GM drum map).
    #[serde(default)]
    pub sf2_bank: u32,
    /// Notes written directly onto this track (via [`Song::add`]), in addition
    /// to any arranged from patterns. `step` is absolute from the song start.
    #[serde(default)]
    pub notes: Vec<SeqNote>,
    /// Voice-specific synthesis parameters (from the catalog instrument).
    #[serde(default)]
    pub voice: VoiceParams,
}

/// A reusable phrase: notes on the bar grid, `bars` long. Note `step`s are
/// relative to the pattern's start, so the same pattern drops in at any bar.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Pattern {
    pub name: String,
    /// Length in bars (how far the next pattern on the same track is pushed).
    pub bars: u32,
    /// The notes, with `step` relative to the pattern start.
    pub notes: Vec<SeqNote>,
}

/// Place `pattern` on `track` starting at bar `bar` (0-based).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Placement {
    pub track: String,
    pub pattern: String,
    pub bar: u32,
}

/// A full song: tracks (instruments), patterns (phrases), and an arrangement
/// (where each pattern plays). Serializable, so a song is a saveable project.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Song {
    pub name: String,
    /// Tempo in beats per minute.
    pub bpm: f32,
    /// Grid resolution: steps per beat (4 = sixteenth notes).
    #[serde(default = "default_steps_per_beat")]
    pub steps_per_beat: u32,
    /// Beats per bar (time-signature numerator; 4 = 4/4).
    #[serde(default = "default_beats_per_bar")]
    pub beats_per_bar: u32,
    /// Swing, 0..1, applied to every track.
    #[serde(default)]
    pub swing: f32,
    /// Humanize, 0..1 (deterministic timing/velocity jitter), applied to every track.
    #[serde(default)]
    pub humanize: f32,
    pub tracks: Vec<SongTrack>,
    pub patterns: Vec<Pattern>,
    pub arrangement: Vec<Placement>,
    /// A master effect chain over the whole mix.
    #[serde(default)]
    pub master: Vec<Node>,
}

/// A note for a pattern at grid `step`, `len` steps long, pitched by name
/// (`"C4"`, `"F#3"`, `"midi:36"`) or Hz — velocity 1.0.
pub fn note(step: u32, len: u32, pitch: &str) -> SeqNote {
    note_vel(step, len, pitch, 1.0)
}

/// [`note`] with an explicit velocity (0..1).
pub fn note_vel(step: u32, len: u32, pitch: &str, gain: f32) -> SeqNote {
    SeqNote {
        step,
        len,
        pitch: Value::Note(pitch.to_string()),
        gain,
    }
}

impl Song {
    /// An empty song at `bpm`, 4/4, sixteenth-note grid.
    pub fn new(name: impl Into<String>, bpm: f32) -> Self {
        Song {
            name: name.into(),
            bpm,
            steps_per_beat: default_steps_per_beat(),
            beats_per_bar: default_beats_per_bar(),
            swing: 0.0,
            humanize: 0.0,
            tracks: Vec::new(),
            patterns: Vec::new(),
            arrangement: Vec::new(),
            master: Vec::new(),
        }
    }

    /// Add an instrument track.
    pub fn add_track(&mut self, name: impl Into<String>, wave: SeqWave, env: Adsr) -> &mut Self {
        self.tracks.push(SongTrack {
            name: name.into(),
            wave,
            env,
            gain: 1.0,
            pan: 0.0,
            sf2: String::new(),
            sf2_preset: 0,
            sf2_bank: 0,
            notes: Vec::new(),
            voice: VoiceParams::default(),
        });
        self
    }

    /// Add a catalog [`Instrument`] and write its notes on the shared beat
    /// timeline, in one fluent call — the ergonomic way to build a song.
    ///
    /// The closure gets a [`Phrase`]: place notes with `.at(beat).note(pitch,
    /// beats)`, step a melody with `.play(..)` / `.rest(..)`, stack a `.chord(..)`,
    /// or hit drums with `.kick()` / `.snare()` / `.hat()`. Consumes and returns
    /// the song so calls chain: `Song::new(..).add(..).add(..).to_doc()`.
    ///
    /// Beats map to the grid at the song's `steps_per_beat`; a duplicate
    /// instrument name is disambiguated automatically.
    pub fn add(mut self, instrument: Instrument, write: impl FnOnce(&mut Phrase)) -> Self {
        let mut phrase = Phrase::new(self.steps_per_beat);
        write(&mut phrase);
        let name = self.unique_name(&instrument.name);
        self.tracks.push(SongTrack {
            name,
            wave: instrument.wave,
            env: instrument.env,
            gain: instrument.gain,
            pan: instrument.pan,
            sf2: String::new(),
            sf2_preset: 0,
            sf2_bank: 0,
            notes: phrase.notes,
            voice: instrument.voice,
        });
        self
    }

    /// A track name not already taken — appends " 2", " 3", … on collision.
    fn unique_name(&self, base: &str) -> String {
        if !self.tracks.iter().any(|t| t.name == base) {
            return base.to_string();
        }
        (2..)
            .map(|i| format!("{base} {i}"))
            .find(|n| !self.tracks.iter().any(|t| &t.name == n))
            .expect("an unused suffix always exists")
    }

    /// Define a reusable pattern.
    pub fn add_pattern(
        &mut self,
        name: impl Into<String>,
        bars: u32,
        notes: Vec<SeqNote>,
    ) -> &mut Self {
        self.patterns.push(Pattern {
            name: name.into(),
            bars: bars.max(1),
            notes,
        });
        self
    }

    /// Place a pattern on a track at `bar`.
    pub fn arrange(&mut self, track: impl Into<String>, pattern: impl Into<String>, bar: u32) {
        self.arrangement.push(Placement {
            track: track.into(),
            pattern: pattern.into(),
            bar,
        });
    }

    /// Place a pattern `times` times back-to-back on a track from `start_bar`
    /// (a repeated section). The pattern's `bars` sets the stride.
    pub fn arrange_repeat(&mut self, track: &str, pattern: &str, start_bar: u32, times: u32) {
        let stride = self
            .patterns
            .iter()
            .find(|p| p.name == pattern)
            .map(|p| p.bars)
            .unwrap_or(1);
        for i in 0..times {
            self.arrange(track, pattern, start_bar + i * stride);
        }
    }

    /// Set the master effect chain (builder style).
    pub fn with_master(mut self, master: Vec<Node>) -> Self {
        self.master = master;
        self
    }

    /// The song's length in bars (the end of its last-ending pattern).
    pub fn length_bars(&self) -> u32 {
        self.arrangement
            .iter()
            .map(|pl| {
                let bars = self
                    .patterns
                    .iter()
                    .find(|p| p.name == pl.pattern)
                    .map(|p| p.bars)
                    .unwrap_or(0);
                pl.bar + bars
            })
            .max()
            .unwrap_or(0)
    }

    /// Compile to a deterministic [`SoundDoc`] — a `tracks` root of `seq` tracks.
    /// Errors if the song is empty or an arrangement references a missing track
    /// or pattern.
    pub fn to_doc(&self) -> Result<SoundDoc, String> {
        if self.tracks.is_empty() {
            return Err("song has no tracks".into());
        }
        for pl in &self.arrangement {
            if !self.tracks.iter().any(|t| t.name == pl.track) {
                return Err(format!(
                    "arrangement references unknown track '{}'",
                    pl.track
                ));
            }
            if !self.patterns.iter().any(|p| p.name == pl.pattern) {
                return Err(format!(
                    "arrangement references unknown pattern '{}'",
                    pl.pattern
                ));
            }
        }

        let steps_per_bar = self.beats_per_bar.max(1) * self.steps_per_beat.max(1);
        let sec_per_step = 60.0 / (self.bpm.max(1.0) * self.steps_per_beat.max(1) as f32);
        let mut end_step = 0u32;
        let mut doc_tracks = Vec::with_capacity(self.tracks.len());

        for t in &self.tracks {
            let mut notes: Vec<SeqNote> = t.notes.clone();
            for n in &notes {
                end_step = end_step.max(n.step + n.len.max(1));
            }
            for pl in self.arrangement.iter().filter(|p| p.track == t.name) {
                let pat = self
                    .patterns
                    .iter()
                    .find(|p| p.name == pl.pattern)
                    .expect("pattern existence checked above");
                let offset = pl.bar * steps_per_bar;
                for n in &pat.notes {
                    let step = n.step + offset;
                    end_step = end_step.max(step + n.len.max(1));
                    notes.push(SeqNote {
                        step,
                        len: n.len,
                        pitch: n.pitch.clone(),
                        gain: n.gain,
                    });
                }
            }
            notes.sort_by_key(|n| n.step);

            // Build the seq node via serde so the seq-only fields (duty, fm_*,
            // pluck_decay) take the engine's own defaults — then override just
            // the ones this instrument sets, so unset params never drift.
            let mut seq_json = serde_json::json!({
                "type": "seq",
                "bpm": self.bpm,
                "steps_per_beat": self.steps_per_beat,
                "wave": serde_json::to_value(t.wave).map_err(|e| e.to_string())?,
                "env": serde_json::to_value(t.env).map_err(|e| e.to_string())?,
                "swing": self.swing,
                "humanize": self.humanize,
                "sf2": t.sf2,
                "sf2_preset": t.sf2_preset,
                "sf2_bank": t.sf2_bank,
                "notes": serde_json::to_value(&notes).map_err(|e| e.to_string())?,
            });
            if let Some(v) = t.voice.duty {
                seq_json["duty"] = serde_json::json!(v);
            }
            if let Some(v) = t.voice.fm_ratio {
                seq_json["fm_ratio"] = serde_json::json!(v);
            }
            if let Some(v) = t.voice.fm_index {
                seq_json["fm_index"] = serde_json::json!(v);
            }
            if let Some(v) = t.voice.fm_strike {
                seq_json["fm_strike"] = serde_json::json!(v);
            }
            if let Some(v) = t.voice.pluck_decay {
                seq_json["pluck_decay"] = serde_json::json!(v);
            }
            if let Some(v) = t.voice.piano_hammer {
                seq_json["piano_hammer"] = serde_json::json!(v);
            }
            if let Some(v) = t.voice.piano_strike {
                seq_json["piano_strike"] = serde_json::json!(v);
            }
            if let Some(v) = t.voice.piano_inharm {
                seq_json["piano_inharm"] = serde_json::json!(v);
            }
            if let Some(v) = t.voice.piano_detune {
                seq_json["piano_detune"] = serde_json::json!(v);
            }
            if let Some(v) = t.voice.piano_decay {
                seq_json["piano_decay"] = serde_json::json!(v);
            }
            if let Some(k) = t.voice.kit {
                seq_json["kit"] = serde_json::to_value(k).map_err(|e| e.to_string())?;
            }
            let seq: Node = serde_json::from_value(seq_json)
                .map_err(|e| format!("track '{}' seq build: {e}", t.name))?;

            doc_tracks.push(Track {
                id: Some(t.name.clone()),
                node: seq,
                pan: t.pan,
                gain: t.gain,
                at: 0.0,
                mute: false,
                automation: Vec::new(),
            });
        }

        let duration = end_step as f32 * sec_per_step + 2.0; // tail for release/reverb
        let root = Node::Tracks {
            tracks: doc_tracks,
            master: self.master.clone(),
        };
        let doc: SoundDoc = serde_json::from_value(serde_json::json!({
            "name": self.name,
            "duration": duration,
            "engine": ENGINE_VERSION,
            "root": serde_json::to_value(&root).map_err(|e| e.to_string())?,
        }))
        .map_err(|e| format!("song doc build: {e}"))?;
        Ok(doc)
    }
}

/// A per-track note writer, handed to the closure of [`Song::add`]. A moving
/// beat cursor plus placement helpers: absolute (`.at(beat)` then `.note(..)` /
/// `.chord(..)`), sequential (`.play(..)` / `.rest(..)` advance the cursor), and
/// drum hits (`.kick()` / `.snare()` / `.hat()` / `.hit(gm_note)`). Velocity for
/// following notes is set with `.vel(..)`.
pub struct Phrase {
    steps_per_beat: u32,
    cursor_beat: f32,
    velocity: f32,
    notes: Vec<SeqNote>,
}

impl Phrase {
    fn new(steps_per_beat: u32) -> Self {
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

fn unit_gain() -> f32 {
    1.0
}
fn default_steps_per_beat() -> u32 {
    4
}
fn default_beats_per_bar() -> u32 {
    4
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::render;

    fn amp() -> Adsr {
        Adsr {
            a: 0.005,
            d: 0.1,
            s: 0.8,
            r: 0.2,
            punch: 0.0,
        }
    }
    fn peak(s: &[f32]) -> f32 {
        s.iter().fold(0.0f32, |m, &x| m.max(x.abs()))
    }

    #[test]
    fn compiles_and_renders_a_two_track_song() {
        let mut song = Song::new("demo", 120.0);
        song.add_track("bass", SeqWave::Bass, amp());
        song.add_track("drums", SeqWave::Kit, amp());
        song.add_pattern("bassline", 1, vec![note(0, 4, "C2"), note(8, 4, "G2")]);
        song.add_pattern(
            "beat",
            1,
            vec![note(0, 2, "midi:36"), note(8, 2, "midi:38")],
        );
        song.arrange_repeat("bass", "bassline", 0, 2);
        song.arrange_repeat("drums", "beat", 0, 2);
        assert_eq!(song.length_bars(), 2);

        let doc = song.to_doc().unwrap();
        assert!(matches!(&doc.root, Node::Tracks { tracks, .. } if tracks.len() == 2));
        let out = render::render(&doc);
        assert!(peak(&out) > 0.0, "the song makes sound");
        // Deterministic: recompiling and re-rendering yields the same samples.
        assert_eq!(render::render(&song.to_doc().unwrap()), out);
    }

    #[test]
    fn pattern_places_at_the_right_bar() {
        // 4/4 at 4 steps/beat ⇒ 16 steps per bar.
        let mut song = Song::new("s", 120.0);
        song.add_track("lead", SeqWave::Square, amp());
        song.add_pattern("p", 1, vec![note(0, 1, "C4")]);
        song.arrange("lead", "p", 0);
        song.arrange("lead", "p", 2); // bar 2 ⇒ step 32
        let doc = song.to_doc().unwrap();
        let Node::Tracks { tracks, .. } = &doc.root else {
            panic!("tracks root");
        };
        let Node::Seq { notes, .. } = &tracks[0].node else {
            panic!("seq track");
        };
        assert_eq!(
            notes.iter().map(|n| n.step).collect::<Vec<_>>(),
            vec![0, 32]
        );
    }

    #[test]
    fn rejects_unknown_references() {
        let mut a = Song::new("s", 120.0);
        a.add_track("t", SeqWave::Sine, amp());
        a.add_pattern("p", 1, vec![note(0, 1, "C4")]);
        a.arrange("nope", "p", 0);
        assert!(a.to_doc().unwrap_err().contains("unknown track"));

        let mut b = Song::new("s", 120.0);
        b.add_track("t", SeqWave::Sine, amp());
        b.arrange("t", "ghost", 0);
        assert!(b.to_doc().unwrap_err().contains("unknown pattern"));
    }

    #[test]
    fn round_trips_through_serde() {
        let mut song = Song::new("s", 128.0);
        song.add_track("bass", SeqWave::Bass, amp());
        song.add_pattern("r", 1, vec![note(0, 4, "C2")]);
        song.arrange("bass", "r", 0);
        let json = serde_json::to_string(&song).unwrap();
        let back: Song = serde_json::from_str(&json).unwrap();
        assert!(back.to_doc().is_ok(), "a saved song reloads and compiles");
    }

    #[test]
    fn fluent_add_places_notes_on_the_beat_grid() {
        use crate::catalog::{Drums, GrandPiano};
        // 4 steps/beat: beat 0 → step 0, beat 1 → step 4, beat 0.5 → step 2.
        let song = Song::new("demo", 120.0)
            .add(GrandPiano::grand(), |t| {
                t.at(0.0).note("C4", 1.0).at(1.0).note("E4", 1.0);
            })
            .add(Drums::acoustic(), |t| {
                t.at(0.0).kick().at(0.5).hat();
            });
        let doc = song.to_doc().unwrap();
        let Node::Tracks { tracks, .. } = &doc.root else {
            panic!("tracks root");
        };
        assert_eq!(tracks.len(), 2);
        let Node::Seq { notes, .. } = &tracks[0].node else {
            panic!("seq");
        };
        assert_eq!(notes.iter().map(|n| n.step).collect::<Vec<_>>(), vec![0, 4]);
        let Node::Seq { notes: drums, .. } = &tracks[1].node else {
            panic!("seq");
        };
        assert_eq!(drums.iter().map(|n| n.step).collect::<Vec<_>>(), vec![0, 2]);
    }

    #[test]
    fn fluent_song_renders_deterministically() {
        use crate::catalog::{Bass, GrandPiano};
        let build = || {
            Song::new("tune", 100.0)
                .add(GrandPiano::grand(), |t| {
                    t.play("C4", 1.0).play("E4", 1.0).play("G4", 1.0);
                })
                .add(Bass::finger(), |t| {
                    t.at(0.0).note("C2", 3.0);
                })
                .to_doc()
                .unwrap()
        };
        let a = render::render(&build());
        assert!(peak(&a) > 0.0, "the fluent song makes sound");
        assert_eq!(render::render(&build()), a, "byte-identical every render");
    }

    #[test]
    fn guitar_voice_param_reaches_the_seq() {
        use crate::catalog::Guitar;
        let doc = Song::new("g", 120.0)
            .add(Guitar::steel(), |t| {
                t.at(0.0).note("E3", 2.0);
            })
            .to_doc()
            .unwrap();
        let Node::Tracks { tracks, .. } = &doc.root else {
            panic!("tracks");
        };
        let Node::Seq { pluck_decay, .. } = &tracks[0].node else {
            panic!("seq");
        };
        assert!((*pluck_decay - 0.965).abs() < 1e-6, "steel pluck_decay set");
    }

    #[test]
    fn duplicate_instrument_names_are_disambiguated() {
        use crate::catalog::GrandPiano;
        let song = Song::new("two pianos", 120.0)
            .add(GrandPiano::grand(), |t| {
                t.at(0.0).note("C4", 1.0);
            })
            .add(GrandPiano::grand(), |t| {
                t.at(0.0).note("E4", 1.0);
            });
        assert_eq!(song.tracks[0].name, "grand piano");
        assert_eq!(song.tracks[1].name, "grand piano 2");
    }
}
