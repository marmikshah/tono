//! studio — the headless project state behind the pattern station.
//!
//! Owns the [`Project`] (a [`Song`] plus the step-grid rows viewing it),
//! snapshot undo/redo, and compilation to an exactly-loopable [`SoundDoc`].
//! The `Song` stays the single source of truth: a grid cell is nothing more
//! than "this track has a note at (step, row pitch)", so the saved project is
//! an ordinary song any face of tono can render. No Tauri, no audio in here —
//! everything is unit-testable.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};
use tono_core::catalog::{Bass, Drums, GrandPiano};
use tono_core::dsl::{SeqNote, SoundDoc, Value, note_to_hz};
use tono_core::song::Song;

/// One grid row: a lane that strikes `pitch` on `track` (drum lanes pick the
/// piece by MIDI pitch; melodic lanes are re-pitchable from the UI).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Row {
    /// Display label ("Kick", "Bass").
    pub label: String,
    /// The song track this lane writes to.
    pub track: String,
    /// The note every cell strikes (`"C2"`, `"midi:36"`).
    pub pitch: String,
    /// Note length in grid steps.
    pub len: u32,
}

/// The saveable project: the song plus the grid rows viewing it. Serialized
/// as-is — the embedded [`Song`] carries its own engine/version pins, so a
/// saved pattern replays byte-identically across engine upgrades.
#[derive(Clone, Serialize, Deserialize)]
pub struct Project {
    /// The music — the single source of truth the grid views.
    pub song: Song,
    /// The grid lanes.
    pub rows: Vec<Row>,
    /// Pattern length in bars.
    pub bars: u32,
    /// Tracks left out of the mix (kept in the song, skipped at compile).
    #[serde(default)]
    pub muted: BTreeSet<String>,
}

impl Project {
    /// The default 8-lane pattern: an acoustic kit, a fingered bass, and a
    /// mellow piano over one 16-step bar.
    pub fn new() -> Project {
        let song = Song::new("pattern", 120.0)
            .add(Drums::acoustic().named("drums"), |_| {})
            .add(Bass::finger().named("bass"), |_| {})
            .add(GrandPiano::mellow().named("keys"), |_| {});
        let lane = |label: &str, track: &str, pitch: &str, len: u32| Row {
            label: label.to_string(),
            track: track.to_string(),
            pitch: pitch.to_string(),
            len,
        };
        Project {
            song,
            rows: vec![
                lane("Kick", "drums", "midi:36", 1),
                lane("Snare", "drums", "midi:38", 1),
                lane("Hat", "drums", "midi:42", 1),
                lane("Open hat", "drums", "midi:46", 1),
                lane("Bass", "bass", "C2", 2),
                lane("Bass 2", "bass", "G1", 2),
                lane("Keys", "keys", "C4", 2),
                lane("Keys 2", "keys", "G4", 2),
            ],
            bars: 1,
            muted: BTreeSet::new(),
        }
    }

    /// Total grid steps (bars × beats × steps per beat).
    pub fn steps(&self) -> u32 {
        self.bars * self.song.beats_per_bar.max(1) * self.song.steps_per_beat.max(1)
    }

    /// The exact loop length in seconds — the compiled doc's duration, so the
    /// buffer wraps seamlessly on the bar line.
    pub fn loop_secs(&self) -> f32 {
        self.steps() as f32 * 60.0
            / (self.song.bpm.max(1.0) * self.song.steps_per_beat.max(1) as f32)
    }

    fn track_index(&self, name: &str) -> Option<usize> {
        self.song.tracks.iter().position(|t| t.name == name)
    }

    /// Whether the row's cell at `step` holds a note.
    pub fn cell(&self, row: &Row, step: u32) -> bool {
        self.track_index(&row.track)
            .map(|t| {
                self.song.tracks[t]
                    .notes
                    .iter()
                    .any(|n| n.step == step && pitch_str(&n.pitch) == row.pitch)
            })
            .unwrap_or(false)
    }

    /// Flip the row's cell at `step` (add or remove the note).
    pub fn toggle(&mut self, row_ix: usize, step: u32) {
        let Some(row) = self.rows.get(row_ix).cloned() else {
            return;
        };
        let Some(t) = self.track_index(&row.track) else {
            return;
        };
        let notes = &mut self.song.tracks[t].notes;
        let existing = notes
            .iter()
            .position(|n| n.step == step && pitch_str(&n.pitch) == row.pitch);
        match existing {
            Some(i) => {
                notes.remove(i);
            }
            None => {
                notes.push(SeqNote {
                    step,
                    len: row.len.max(1),
                    pitch: Value::Note(row.pitch.clone()),
                    gain: 0.9,
                });
                notes.sort_by_key(|n| n.step);
            }
        }
    }

    /// Re-pitch a melodic lane: the row and every note it owns move together.
    /// Rejected (no-op, returns false) if `pitch` isn't a valid note name.
    pub fn set_row_pitch(&mut self, row_ix: usize, pitch: &str) -> bool {
        if note_to_hz(pitch).is_none() {
            return false;
        }
        let Some(row) = self.rows.get(row_ix).cloned() else {
            return false;
        };
        if let Some(t) = self.track_index(&row.track) {
            for n in self.song.tracks[t].notes.iter_mut() {
                if pitch_str(&n.pitch) == row.pitch {
                    n.pitch = Value::Note(pitch.to_string());
                }
            }
        }
        self.rows[row_ix].pitch = pitch.to_string();
        true
    }

    /// Compile the pattern to an exactly-loopable doc: muted and empty tracks
    /// are skipped, and the duration is pinned to the bar line (`to_doc`'s
    /// ring-out tail would break the seam). `None` when the grid is silent.
    pub fn loop_doc(&self) -> Result<Option<SoundDoc>, String> {
        let mut song = self.song.clone();
        song.tracks
            .retain(|t| !t.notes.is_empty() && !self.muted.contains(&t.name));
        if song.tracks.is_empty() {
            return Ok(None);
        }
        let mut doc = song.to_doc()?;
        doc.duration = self.loop_secs();
        doc.ensure_track_ids();
        doc.validate()?;
        Ok(Some(doc))
    }
}

impl Default for Project {
    fn default() -> Self {
        Project::new()
    }
}

/// A note's pitch as the grid's comparison key (rows store note names).
fn pitch_str(v: &Value) -> String {
    match v {
        Value::Note(s) => s.clone(),
        Value::Const(c) => format!("{c}"),
        Value::Modulated(_) => String::new(),
    }
}

/// The station: the live project plus snapshot undo/redo. Snapshots are whole
/// [`Project`] clones — a pattern is a few kilobytes, so this is the simple
/// thing that is also fast enough.
pub struct Station {
    /// The live project.
    pub project: Project,
    undo: Vec<Project>,
    redo: Vec<Project>,
}

/// Undo depth — enough for a whole session of grid pokes.
const UNDO_CAP: usize = 100;

impl Station {
    /// A fresh station on the default pattern.
    pub fn new() -> Station {
        Station {
            project: Project::new(),
            undo: Vec::new(),
            redo: Vec::new(),
        }
    }

    /// Run `change` against the project with an undo snapshot taken first.
    pub fn edit(&mut self, change: impl FnOnce(&mut Project)) {
        self.undo.push(self.project.clone());
        if self.undo.len() > UNDO_CAP {
            self.undo.remove(0);
        }
        self.redo.clear();
        change(&mut self.project);
    }

    /// Step back one edit. Returns false at the bottom of the stack.
    pub fn undo(&mut self) -> bool {
        match self.undo.pop() {
            Some(prev) => {
                self.redo.push(std::mem::replace(&mut self.project, prev));
                true
            }
            None => false,
        }
    }

    /// Re-apply the last undone edit.
    pub fn redo(&mut self) -> bool {
        match self.redo.pop() {
            Some(next) => {
                self.undo.push(std::mem::replace(&mut self.project, next));
                true
            }
            None => false,
        }
    }

    /// Whether undo/redo have anything to pop (for the UI's button state).
    pub fn depths(&self) -> (usize, usize) {
        (self.undo.len(), self.redo.len())
    }

    /// Save the project as JSON at `path`.
    pub fn save(&self, path: &str) -> Result<(), String> {
        let json = serde_json::to_string_pretty(&self.project).map_err(|e| e.to_string())?;
        std::fs::write(expand_home(path), json).map_err(|e| e.to_string())
    }

    /// Load a project from `path`, replacing the current one (undoable).
    pub fn load(&mut self, path: &str) -> Result<(), String> {
        let json = std::fs::read_to_string(expand_home(path)).map_err(|e| e.to_string())?;
        let project: Project = serde_json::from_str(&json).map_err(|e| e.to_string())?;
        self.edit(|p| *p = project);
        Ok(())
    }
}

impl Default for Station {
    fn default() -> Self {
        Station::new()
    }
}

/// `~/` paths expand against $HOME so the save box takes the obvious spelling.
fn expand_home(path: &str) -> String {
    match (path.strip_prefix("~/"), std::env::var("HOME")) {
        (Some(rest), Ok(home)) => format!("{home}/{rest}"),
        _ => path.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_pattern_has_lanes_over_real_tracks() {
        let p = Project::new();
        assert_eq!(p.steps(), 16);
        for row in &p.rows {
            assert!(
                p.song.tracks.iter().any(|t| t.name == row.track),
                "row '{}' points at a real track",
                row.label
            );
        }
        // Empty grid: nothing to play.
        assert!(p.loop_doc().unwrap().is_none());
    }

    #[test]
    fn toggle_writes_and_erases_song_notes() {
        let mut p = Project::new();
        p.toggle(0, 0); // kick on the downbeat
        p.toggle(4, 8); // bass mid-bar
        assert!(p.cell(&p.rows[0].clone(), 0));
        assert!(p.cell(&p.rows[4].clone(), 8));
        assert_eq!(p.song.tracks[0].notes.len(), 1);
        p.toggle(0, 0);
        assert!(!p.cell(&p.rows[0].clone(), 0));
        assert!(p.song.tracks[0].notes.is_empty());
    }

    #[test]
    fn loop_doc_is_exactly_one_bar_and_skips_muted_and_empty() {
        let mut p = Project::new();
        p.toggle(0, 0);
        p.toggle(4, 0);
        let doc = p.loop_doc().unwrap().expect("two lanes sound");
        // 16 steps at 120 bpm, 4 steps/beat → exactly 2 s, no ring-out tail.
        assert!((doc.duration - 2.0).abs() < 1e-6);
        // Only the two non-empty tracks compile (keys is empty).
        let tono_core::dsl::Node::Tracks { tracks, .. } = &doc.root else {
            panic!("tracks root");
        };
        assert_eq!(tracks.len(), 2);
        // Muting the bass drops it from the mix but keeps its notes.
        p.muted.insert("bass".into());
        let doc = p.loop_doc().unwrap().unwrap();
        let tono_core::dsl::Node::Tracks { tracks, .. } = &doc.root else {
            panic!("tracks root");
        };
        assert_eq!(tracks.len(), 1);
        assert!(!p.song.tracks[1].notes.is_empty(), "notes survive the mute");
    }

    #[test]
    fn row_repitch_moves_its_notes() {
        let mut p = Project::new();
        p.toggle(4, 0);
        assert!(p.set_row_pitch(4, "D2"));
        assert!(p.cell(&p.rows[4].clone(), 0), "note follows the lane");
        assert_eq!(pitch_str(&p.song.tracks[1].notes[0].pitch), "D2");
        assert!(!p.set_row_pitch(4, "nonsense"), "bad names are rejected");
        assert_eq!(p.rows[4].pitch, "D2");
    }

    #[test]
    fn undo_redo_walk_the_snapshots() {
        let mut s = Station::new();
        s.edit(|p| p.toggle(0, 0));
        s.edit(|p| p.toggle(0, 4));
        assert_eq!(s.project.song.tracks[0].notes.len(), 2);
        assert!(s.undo());
        assert_eq!(s.project.song.tracks[0].notes.len(), 1);
        assert!(s.redo());
        assert_eq!(s.project.song.tracks[0].notes.len(), 2);
        assert!(s.undo() && s.undo());
        assert!(!s.undo(), "stack bottom");
        // A fresh edit clears the redo branch.
        s.edit(|p| p.toggle(1, 2));
        assert!(!s.redo());
    }

    #[test]
    fn project_round_trips_through_json() {
        let mut p = Project::new();
        p.toggle(0, 0);
        p.set_row_pitch(4, "E2");
        p.muted.insert("keys".into());
        let json = serde_json::to_string(&p).unwrap();
        let back: Project = serde_json::from_str(&json).unwrap();
        assert!(back.cell(&back.rows[0].clone(), 0));
        assert_eq!(back.rows[4].pitch, "E2");
        assert!(back.muted.contains("keys"));
        // The song inside carries its engine pin — saved patterns replay
        // byte-identically across kernel upgrades.
        assert_eq!(back.song.engine, p.song.engine);
    }
}
