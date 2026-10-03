//! song — compose a full piece by adding instruments and arranging parts.
//!
//! A [`Song`] is the ergonomic layer above the raw graph: you add instrument
//! **tracks**, define reusable **patterns** (phrases on a bar grid), and
//! **arrange** them on a timeline. [`Song::to_doc`] compiles the whole thing
//! down to an ordinary [`SoundDoc`](crate::dsl::SoundDoc) (a `tracks` root of `seq` tracks), so it
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

mod compile;
mod diagnostics;
mod estimate;
mod pattern;
mod phrase;

pub use compile::{CompileOptions, CompileTarget};
pub use pattern::{
    PatternError, concat, euclidean, gate, humanize, layer, probability, quantize, repeat, reverse,
    rotate, slice, stretch, transpose, tuplet, vel,
};
pub use phrase::Phrase;

use serde::{Deserialize, Serialize};

use crate::catalog::{Voice, VoiceParams};
use crate::dsl::{Adsr, ENGINE_VERSION, Node, SeqNote, SeqWave, Value};

/// One instrument track: an instrument voice plus its mixer settings. Notes come
/// from the patterns arranged onto it.
#[non_exhaustive]
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
    /// Reverb send, 0..1 — wraps the track's seq in a reverb (0 = dry).
    #[serde(default)]
    pub reverb: f32,
    /// Per-track swing override (0..1); `None` uses the song's swing.
    #[serde(default)]
    pub swing: Option<f32>,
    /// Per-track humanize override (0..1); `None` uses the song's humanize.
    #[serde(default)]
    pub humanize: Option<f32>,
    /// Muted tracks compile to a muted mixer layer (present but silent).
    #[serde(default)]
    pub mute: bool,
    /// When ANY track is solo, every non-solo track is muted — the console
    /// behavior, deterministic regardless of declaration order. A track that
    /// is both muted and solo stays muted.
    #[serde(default)]
    pub solo: bool,
    /// Automation on this track's gain/pan, addressed in BEATS on the song
    /// grid and compiled to seconds through the tempo map (or the constant
    /// bpm). Empty = the static `gain`/`pan` apply — byte-identical.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub automation: Vec<SongLane>,
    /// The mix bus this track routes to (a name from the song's `buses`).
    /// None = the master bus.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bus: Option<String>,
    /// Post-fader sends into the song's mix buses.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sends: Vec<crate::dsl::Send>,
}

/// One automation lane on a song track: `target` driven by beat-addressed
/// breakpoints (see the document's `AutoLane` for the compiled form).
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct SongLane {
    /// What this lane controls (gain or pan).
    pub target: crate::dsl::AutoTarget,
    /// The interpolation between breakpoints (default linear).
    #[serde(default)]
    pub curve: crate::dsl::AutoCurve,
    /// Breakpoints on the beat grid.
    pub points: Vec<SongPoint>,
}

/// One beat-addressed automation breakpoint.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq)]
pub struct SongPoint {
    /// The beat position (fractional beats are fine — the lane is a
    /// continuous curve, not grid events).
    pub at: f32,
    /// The target value at this beat.
    pub v: f32,
}

/// A reusable phrase: notes on the bar grid, `bars` long. Note `step`s are
/// relative to the pattern's start, so the same pattern drops in at any bar.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Pattern {
    /// Pattern name — placements reference it.
    pub name: String,
    /// Length in bars (how far the next pattern on the same track is pushed).
    pub bars: u32,
    /// The notes, with `step` relative to the pattern start.
    pub notes: Vec<SeqNote>,
}

/// Place `pattern` on `track` starting at bar `bar` (0-based).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Placement {
    /// The track the pattern plays on.
    pub track: String,
    /// The pattern to play.
    pub pattern: String,
    /// The bar it starts at (0-based).
    pub bar: u32,
}

pub use crate::units::MeterPoint;

/// A named range of bars — a verse, a chorus, a build. Sections are musical
/// metadata: they render nothing themselves, but they are compiled into the
/// Program so the runtime can quantize transitions to them.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Section {
    /// The section name (`"verse"`, `"chorus"`).
    pub name: String,
    /// The bar it starts at (0-based).
    pub bar: u32,
    /// Its length in bars.
    pub bars: u32,
}

/// A named point on the musical timeline — a hit, a cue, a drop. Like
/// sections, markers are metadata compiled into the Program.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Marker {
    /// The marker name.
    pub name: String,
    /// Its exact beat position.
    pub at: crate::units::Beat,
}

/// A full song: tracks (instruments), patterns (phrases), and an arrangement
/// (where each pattern plays). Serializable, so a song is a saveable project.
#[non_exhaustive]
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Song {
    /// Project name.
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
    /// Tempo changes at exact beat positions. Empty = the constant
    /// `bpm`. The first point must sit at beat 0.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tempo_map: Vec<crate::dsl::TempoPoint>,
    /// Time-signature changes by bar. Empty = `beats_per_bar`/4 throughout.
    /// The first point must be bar 0 when present.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub meter_map: Vec<MeterPoint>,
    /// Pickup (anacrusis): bar 0's length in beats when it isn't a full bar.
    /// None = bar 0 is full length.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pickup: Option<crate::units::Beat>,
    /// Named ranges of bars (verse/chorus/…) — metadata compiled into the
    /// Program for the runtime's quantized transitions.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sections: Vec<Section>,
    /// Named points on the timeline — metadata compiled into the Program.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub markers: Vec<Marker>,
    /// Mix buses: named submixes with insert chains (e.g. a shared reverb).
    /// Tracks route to them with their `bus` field and feed them with `sends`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub buses: Vec<crate::dsl::Bus>,
    /// The instrument tracks.
    pub tracks: Vec<SongTrack>,
    /// The reusable phrases.
    pub patterns: Vec<Pattern>,
    /// Where each pattern plays.
    pub arrangement: Vec<Placement>,
    /// A master effect chain over the whole mix.
    #[serde(default)]
    pub master: Vec<Node>,
    /// Song-level deterministic seed, stamped onto the compiled document's
    /// `seed` (the RNG stream everything stochastic draws from). `None` keeps
    /// the document default (0). Same song + same seed ⇒ same program hash.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seed: Option<u64>,
    /// The supported DSP-kernel revision, stamped on the compiled document.
    #[serde(default = "crate::dsl::default_engine")]
    pub engine: u32,
    /// The supported document schema revision.
    #[serde(default = "crate::dsl::default_version")]
    pub version: u32,
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

/// Why a song failed to compile to a [`SoundDoc`](crate::dsl::SoundDoc).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SongError {
    /// The song has no tracks.
    Empty,
    /// The arrangement places a pattern on a track that doesn't exist.
    UnknownTrack(String),
    /// The arrangement references a pattern that doesn't exist.
    UnknownPattern(String),
    /// The compiled document failed to build or validate.
    Compile(String),
}

impl std::fmt::Display for SongError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SongError::Empty => f.write_str("song has no tracks"),
            SongError::UnknownTrack(t) => {
                write!(f, "arrangement references unknown track '{t}'")
            }
            SongError::UnknownPattern(p) => {
                write!(f, "arrangement references unknown pattern '{p}'")
            }
            SongError::Compile(e) => f.write_str(e),
        }
    }
}

impl std::error::Error for SongError {}

impl From<SongError> for String {
    fn from(e: SongError) -> String {
        e.to_string()
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
            tempo_map: Vec::new(),
            meter_map: Vec::new(),
            pickup: None,
            sections: Vec::new(),
            markers: Vec::new(),
            buses: Vec::new(),
            tracks: Vec::new(),
            patterns: Vec::new(),
            arrangement: Vec::new(),
            master: Vec::new(),
            seed: None,
            engine: ENGINE_VERSION,
            version: crate::dsl::SCHEMA_VERSION,
        }
    }

    /// Set the song-level deterministic seed (builder style) — see
    /// [`Song::seed`].
    pub fn with_seed(mut self, seed: u64) -> Self {
        self.seed = Some(seed);
        self
    }

    /// Add an instrument track. The name becomes the rendered layer id, so it
    /// is slugified and deduplicated exactly like the fluent [`add`](Self::add)
    /// path — a duplicate name would land a placement on BOTH tracks, and a
    /// non-slug name would fail validation downstream at render.
    pub fn add_track(&mut self, name: impl Into<String>, wave: SeqWave, env: Adsr) -> &mut Self {
        let name = self.unique_name(&slugify(&name.into()));
        self.tracks.push(SongTrack {
            name,
            wave,
            env,
            gain: 1.0,
            pan: 0.0,
            sf2: String::new(),
            sf2_preset: 0,
            sf2_bank: 0,
            notes: Vec::new(),
            voice: VoiceParams::default(),
            reverb: 0.0,
            swing: None,
            humanize: None,
            mute: false,
            solo: false,
            automation: Vec::new(),
            bus: None,
            sends: Vec::new(),
        });
        self
    }

    /// Add a catalog [`Voice`] and write its notes on the shared beat
    /// timeline, in one fluent call — the ergonomic way to build a song.
    ///
    /// The closure gets a [`Phrase`]: place notes with `.at(beat).note(pitch,
    /// beats)`, step a melody with `.play(..)` / `.rest(..)`, stack a `.chord(..)`,
    /// or hit drums with `.kick()` / `.snare()` / `.hat()`. Consumes and returns
    /// the song so calls chain: `Song::new(..).add(..).add(..).to_doc()`.
    ///
    /// Beats map to the grid at the song's `steps_per_beat`; a duplicate
    /// instrument name is disambiguated automatically.
    pub fn add(mut self, instrument: Voice, write: impl FnOnce(&mut Phrase)) -> Self {
        let mut phrase = Phrase::new(self.steps_per_beat);
        write(&mut phrase);
        // The track name becomes the rendered layer id, which must be a slug
        // (a-z, 0-9, _) — so slugify the instrument's display name.
        let name = self.unique_name(&slugify(&instrument.name));
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
            reverb: instrument.reverb,
            swing: instrument.swing,
            humanize: instrument.humanize,
            mute: false,
            solo: false,
            automation: Vec::new(),
            bus: None,
            sends: Vec::new(),
        });
        self
    }

    /// Add a catalog [`Voice`] as a track with an explicit name, writing no
    /// notes — the constructor the Python typed API uses: naming the track
    /// explicitly keeps layer ids stable across faces (the fluent
    /// [`add`](Self::add) path names the track after the instrument's display
    /// name instead). The name is slugified and deduplicated exactly like
    /// [`add_track`](Self::add_track), so patterns arrange onto it and it
    /// becomes the rendered layer id. Notes come from the patterns arranged
    /// onto the track.
    ///
    pub fn add_voice(&mut self, name: impl Into<String>, voice: &Voice) -> &mut Self {
        let name = self.unique_name(&slugify(&name.into()));
        self.tracks.push(SongTrack {
            name,
            wave: voice.wave,
            env: voice.env,
            gain: voice.gain,
            pan: voice.pan,
            sf2: String::new(),
            sf2_preset: 0,
            sf2_bank: 0,
            notes: Vec::new(),
            voice: voice.voice,
            reverb: voice.reverb,
            swing: voice.swing,
            humanize: voice.humanize,
            mute: false,
            solo: false,
            automation: Vec::new(),
            bus: None,
            sends: Vec::new(),
        });
        self
    }

    /// A track name not already taken — appends `_2`, `_3`, … on collision
    /// (keeping it a valid layer-id slug).
    fn unique_name(&self, base: &str) -> String {
        if !self.tracks.iter().any(|t| t.name == base) {
            return base.to_string();
        }
        (2..)
            .map(|i| format!("{base}_{i}"))
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
}

/// Turn a display name into a layer-id slug: lowercase, runs of non-`[a-z0-9]`
/// collapsed to a single `_`, no leading/trailing `_`. `"Mellow Piano"` →
/// `"mellow_piano"`, `"808 drums"` → `"808_drums"`.
fn slugify(name: &str) -> String {
    let mut s = String::with_capacity(name.len());
    let mut pending_us = false;
    for c in name.chars() {
        if c.is_ascii_alphanumeric() {
            if pending_us && !s.is_empty() {
                s.push('_');
            }
            s.push(c.to_ascii_lowercase());
            pending_us = false;
        } else {
            pending_us = true;
        }
    }
    if s.is_empty() {
        s.push_str("track");
    }
    s
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
        assert_eq!(
            a.to_doc().unwrap_err(),
            SongError::UnknownTrack("nope".into())
        );

        let mut b = Song::new("s", 120.0);
        b.add_track("t", SeqWave::Sine, amp());
        b.arrange("t", "ghost", 0);
        assert_eq!(
            b.to_doc().unwrap_err(),
            SongError::UnknownPattern("ghost".into())
        );
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
        let Node::Seq { pluck, .. } = &tracks[0].node else {
            panic!("seq");
        };
        assert!(
            (pluck.pluck_decay - 0.965).abs() < 1e-6,
            "steel pluck_decay set"
        );
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
        assert_eq!(song.tracks[0].name, "grand_piano");
        assert_eq!(song.tracks[1].name, "grand_piano_2");
    }

    #[test]
    fn per_track_reverb_wraps_and_is_dry_by_default() {
        use crate::catalog::GrandPiano;
        // Dry (default): the track node is a bare seq — byte-identical to before.
        let dry = Song::new("s", 100.0)
            .add(GrandPiano::grand(), |t| {
                t.at(0.0).note("C4", 1.0);
            })
            .to_doc()
            .unwrap();
        let Node::Tracks { tracks, .. } = &dry.root else {
            panic!("tracks")
        };
        assert!(
            matches!(&tracks[0].node, Node::Seq { .. }),
            "dry = bare seq"
        );
        // Wet: the seq is wrapped in a chain [seq, reverb].
        let wet = Song::new("s", 100.0)
            .add(GrandPiano::grand().reverb(0.5), |t| {
                t.at(0.0).note("C4", 1.0);
            })
            .to_doc()
            .unwrap();
        let Node::Tracks { tracks, .. } = &wet.root else {
            panic!("tracks")
        };
        let Node::Chain { stages } = &tracks[0].node else {
            panic!("reverb wraps the seq in a chain")
        };
        assert!(matches!(stages[0], Node::Seq { .. }));
        assert!(matches!(stages[1], Node::Reverb { .. }));
        assert!(
            render::render(&wet).iter().any(|&x| x != 0.0),
            "wet song sounds"
        );
    }

    #[test]
    fn per_track_swing_overrides_the_song_swing() {
        use crate::catalog::Bass;
        let doc = Song::new("s", 120.0) // song swing defaults to 0
            .add(Bass::finger().swing(0.6), |t| {
                t.at(0.0).note("C2", 1.0).at(1.0).note("G1", 1.0);
            })
            .to_doc()
            .unwrap();
        let Node::Tracks { tracks, .. } = &doc.root else {
            panic!("tracks")
        };
        let Node::Seq { swing, .. } = &tracks[0].node else {
            panic!("seq")
        };
        assert!(
            (*swing - 0.6).abs() < 1e-6,
            "track swing overrides the song's"
        );
    }

    #[test]
    fn catalog_names_become_valid_layer_id_slugs() {
        use crate::catalog::{Drums, Guitar, Strings};
        // Instruments with spaces / digits in their display names must yield
        // slug layer ids so the doc passes validation (the CLI enforces it).
        let doc = Song::new("s", 100.0)
            .add(Strings::warm(), |t| {
                t.at(0.0).chord(&["C4", "E4"], 4.0);
            })
            .add(Guitar::steel(), |t| {
                t.at(0.0).note("E3", 4.0);
            })
            .add(Drums::tr808(), |t| {
                t.at(0.0).kick();
            })
            .to_doc()
            .unwrap();
        assert!(
            doc.validate().is_ok(),
            "catalog song validates: {:?}",
            doc.validate()
        );
        let Node::Tracks { tracks, .. } = &doc.root else {
            panic!("tracks");
        };
        for t in tracks {
            let id = t.id.as_deref().unwrap();
            assert!(
                id.chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_'),
                "layer id '{id}' is a slug"
            );
        }
    }

    #[test]
    fn song_uses_the_current_format_and_roundtrips() {
        let mut song = Song::new("pinned", 120.0);
        song.add_track("bass", SeqWave::Bass, amp());
        song.tracks[0].notes.push(note(0, 4, "C2"));
        assert_eq!(song.engine, ENGINE_VERSION);
        assert_eq!(song.version, crate::dsl::SCHEMA_VERSION);
        let doc = song.to_doc().unwrap();
        let saved = serde_json::to_string(&song).unwrap();
        let back: Song = serde_json::from_str(&saved).unwrap();
        assert_eq!(
            serde_json::to_string(&doc).unwrap(),
            serde_json::to_string(&back.to_doc().unwrap()).unwrap()
        );
        song.engine = ENGINE_VERSION - 1;
        assert!(song.compile(&CompileOptions::default()).is_err());
    }

    #[test]
    fn length_bars_counts_direct_track_notes() {
        // Notes written via the fluent path live on the track, not in a
        // pattern placement — they must still count toward the song length.
        let mut song = Song::new("fluent", 120.0); // 16 steps per bar
        song.add_track("bass", SeqWave::Bass, amp());
        assert_eq!(song.length_bars(), 0);
        song.tracks[0].notes.push(note(17, 4, "C2")); // ends at step 21 → bar 2
        assert_eq!(song.length_bars(), 2);
    }

    #[test]
    fn slugifies_names() {
        assert_eq!(slugify("Mellow Piano"), "mellow_piano");
        assert_eq!(slugify("808 drums"), "808_drums");
        assert_eq!(slugify("steel guitar"), "steel_guitar");
        assert_eq!(slugify("  !!  "), "track");
    }

    #[test]
    fn pathological_step_values_saturate_instead_of_wrapping() {
        // u32 arithmetic on raw input used to panic in debug and wrap in
        // release; a saturated tail end is harmless (the seq renderer caps
        // notes at the render window anyway).
        let mut song = Song::new("s", 120.0);
        song.add_track("t", SeqWave::Square, amp());
        song.tracks[0].notes.push(note(u32::MAX, 4, "C4"));
        let doc = song.to_doc().unwrap(); // must not panic
        assert!(doc.duration.is_finite());
        // A huge placement bar takes the same path.
        let mut song = Song::new("s", 120.0);
        song.add_track("t", SeqWave::Square, amp());
        song.add_pattern("p", 1, vec![note(0, 1, "C4")]);
        song.arrange("t", "p", u32::MAX);
        let doc = song.to_doc().unwrap();
        assert!(doc.duration.is_finite());
    }

    #[test]
    fn add_track_slugifies_and_dedups_names() {
        // Duplicate names used to land a placement on BOTH tracks; non-slug
        // names compiled to a doc that fails validation downstream.
        let mut song = Song::new("s", 120.0);
        song.add_track("My Bass", SeqWave::Bass, amp());
        song.add_track("My Bass", SeqWave::Bass, amp());
        assert_eq!(song.tracks[0].name, "my_bass");
        assert_eq!(song.tracks[1].name, "my_bass_2");
    }

    #[test]
    fn add_voice_slugifies_dedups_and_carries_the_voice_fields() {
        use crate::catalog::{Bass, Drums};
        let voice = Bass::pick()
            .gain(0.8)
            .pan(-0.25)
            .reverb(0.4)
            .swing(0.5)
            .humanize(0.1);
        let mut song = Song::new("s", 120.0);
        song.add_voice("My Bass", &voice);
        song.add_voice("My Bass", &Bass::finger());
        song.add_voice("drums", &Drums::tr808());
        assert_eq!(song.tracks[0].name, "my_bass");
        assert_eq!(song.tracks[1].name, "my_bass_2");
        assert_eq!(song.tracks[2].name, "drums");

        let t = &song.tracks[0];
        assert_eq!(t.wave, voice.wave);
        assert_eq!(t.env, voice.env);
        assert_eq!(t.gain, 0.8);
        assert_eq!(t.pan, -0.25);
        assert_eq!(t.reverb, 0.4);
        assert_eq!(t.swing, Some(0.5));
        assert_eq!(t.humanize, Some(0.1));
        assert_eq!(t.voice, voice.voice, "the pick's bass_* params ride along");
        assert!(t.notes.is_empty(), "add_voice writes no notes");
        assert!(t.sf2.is_empty() && t.sf2_preset == 0 && t.sf2_bank == 0);
        assert!(!t.mute && !t.solo);

        // The explicitly-named track arranges and compiles like any other.
        song.add_pattern("p", 1, vec![note(0, 2, "C2")]);
        song.arrange("my_bass", "p", 0);
        assert!(song.to_doc().is_ok());
    }

    #[test]
    fn degenerate_bpm_keeps_duration_and_placement_consistent() {
        // bpm < 1 used to size the duration for bpm=1 while the seq played at
        // the real bpm — notes past bar 0 silently dropped. Both use the
        // clamped value now.
        let mut song = Song::new("s", 0.5);
        song.add_track("t", SeqWave::Sine, amp());
        song.tracks[0].notes.push(note(0, 2, "C4"));
        song.tracks[0].notes.push(note(16, 2, "C4")); // ends at step 18
        let doc = song.to_doc().unwrap();
        doc.validate().unwrap();
        let Node::Tracks { tracks, .. } = &doc.root else {
            panic!("tracks root");
        };
        let Node::Seq { bpm, .. } = &tracks[0].node else {
            panic!("a reverb-less track compiles to a bare seq");
        };
        assert_eq!(*bpm, 1.0, "the seq plays at the clamped bpm");
        let expected = 18.0 * (60.0 / 4.0) + 2.0; // 15 s per step at bpm 1
        assert!(
            (doc.duration - expected).abs() < 1e-3,
            "duration matches the clamped bpm: {}",
            doc.duration
        );
    }
}
