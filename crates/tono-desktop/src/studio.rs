//! studio — the headless project state behind the pattern station.
//!
//! Owns the [`Project`] (a [`Song`] plus the step-grid rows viewing it),
//! snapshot undo/redo, and compilation to an exactly-loopable [`SoundDoc`].
//! The `Song` stays the single source of truth: a grid cell is nothing more
//! than "this track has a note at (step, row pitch)", so the saved project is
//! an ordinary song any face of tono can render. No Tauri, no audio in here —
//! everything is unit-testable.

use std::collections::BTreeSet;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

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
/// as-is; the embedded [`Song`] must use the current engine and schema.
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Project {
    /// The music — the single source of truth the grid views.
    pub song: Song,
    /// The grid lanes.
    pub rows: Vec<Row>,
    /// Pattern length in bars.
    pub bars: u32,
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
        }
    }

    /// Total grid steps (bars × beats × steps per beat).
    pub fn steps(&self) -> u32 {
        self.bars
            .saturating_mul(self.song.beats_per_bar)
            .saturating_mul(self.song.steps_per_beat)
    }

    /// Check the current song and the grid's references before replacing live state.
    fn validate(&self) -> Result<(), String> {
        let steps = self
            .bars
            .checked_mul(self.song.beats_per_bar)
            .and_then(|n| n.checked_mul(self.song.steps_per_beat))
            .filter(|&n| n > 0 && n <= 4096)
            .ok_or("pattern grid must have 1..4096 steps")?;
        if self.rows.is_empty()
            || self.rows.len() > 128
            || self.rows.len() * steps as usize > 65_536
        {
            return Err("pattern grid must have 1..128 rows and at most 65536 cells".into());
        }
        if !(30.0..=300.0).contains(&self.song.bpm) || self.loop_secs() > 600.0 {
            return Err(
                "pattern tempo must be 30..300 BPM and its loop at most 600 seconds".into(),
            );
        }
        let mut lanes = BTreeSet::new();
        for row in &self.rows {
            if self.track_index(&row.track).is_none()
                || note_to_hz(&row.pitch).is_none()
                || row.len == 0
                || row.len > steps
                || !lanes.insert((&row.track, &row.pitch))
            {
                return Err(format!("invalid or duplicate grid lane '{}'", row.label));
            }
        }
        if self
            .song
            .tracks
            .iter()
            .flat_map(|t| &t.notes)
            .any(|n| n.step >= steps || n.len == 0 || n.len > steps)
        {
            return Err("pattern notes must lie on the grid".into());
        }
        // Empty lanes are valid projects. Silent notes in the validation clone
        // let the compiler check their voices and routing as well.
        let mut song = self.song.clone();
        for track in &mut song.tracks {
            if track.notes.is_empty() {
                track.notes.push(SeqNote {
                    step: 0,
                    len: 1,
                    pitch: Value::Note("C4".into()),
                    gain: 0.0,
                });
            }
        }
        song.compile(&Default::default())
            .map_err(|e| e.to_string())?;
        Ok(())
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
                    .any(|n| n.step == step && has_pitch(&n.pitch, &row.pitch))
            })
            .unwrap_or(false)
    }

    /// Flip the row's cell at `step` (add or remove the note).
    pub fn toggle(&mut self, row_ix: usize, step: u32) {
        if step >= self.steps() {
            return;
        }
        let Some(row) = self.rows.get(row_ix).cloned() else {
            return;
        };
        let Some(t) = self.track_index(&row.track) else {
            return;
        };
        let notes = &mut self.song.tracks[t].notes;
        let existing = notes
            .iter()
            .position(|n| n.step == step && has_pitch(&n.pitch, &row.pitch));
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
        if self
            .rows
            .iter()
            .enumerate()
            .any(|(i, r)| i != row_ix && r.track == row.track && r.pitch == pitch)
        {
            return false;
        }
        if let Some(t) = self.track_index(&row.track) {
            for n in self.song.tracks[t].notes.iter_mut() {
                if has_pitch(&n.pitch, &row.pitch) {
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
        song.tracks.retain(|t| !t.notes.is_empty() && !t.mute);
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

fn has_pitch(value: &Value, pitch: &str) -> bool {
    matches!(value, Value::Note(name) if name == pitch)
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

    /// Save atomically: a failed write keeps the previous file intact.
    pub fn save(&self, path: &str) -> Result<(), String> {
        self.project.validate()?;
        let path = expand_home(path);
        let parent = path
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        let json = serde_json::to_string_pretty(&self.project).map_err(|e| e.to_string())?;
        let mut file = tempfile::NamedTempFile::new_in(parent).map_err(|e| e.to_string())?;
        file.write_all(json.as_bytes()).map_err(|e| e.to_string())?;
        file.persist(path).map_err(|e| e.error.to_string())?;
        Ok(())
    }

    /// Load a validated project. Unreadable paths preserve the working project;
    /// unsupported/corrupt content is replaced by a fresh current project.
    pub fn load(&mut self, path: &str) -> Result<Option<String>, String> {
        const MAX_BYTES: u64 = 8 * 1024 * 1024;
        let file = std::fs::File::open(expand_home(path)).map_err(|e| e.to_string())?;
        let mut bytes = Vec::new();
        file.take(MAX_BYTES + 1)
            .read_to_end(&mut bytes)
            .map_err(|e| e.to_string())?;
        let loaded = if bytes.len() as u64 > MAX_BYTES {
            Err("project file exceeds 8 MiB".into())
        } else {
            String::from_utf8(bytes)
                .map_err(|e| e.to_string())
                .and_then(|json| serde_json::from_str::<Project>(&json).map_err(|e| e.to_string()))
                .and_then(|project| {
                    project.validate()?;
                    Ok(project)
                })
        };
        match loaded {
            Ok(project) => {
                self.edit(|p| *p = project);
                Ok(None)
            }
            Err(error) => {
                self.edit(|p| *p = Project::new());
                let saved = self.save(path);
                Ok(Some(match saved {
                    Ok(()) => format!("{error}; replaced with a fresh current project"),
                    Err(write) => format!(
                        "{error}; started a fresh project but could not replace the file: {write}"
                    ),
                }))
            }
        }
    }
}

impl Default for Station {
    fn default() -> Self {
        Station::new()
    }
}

/// Expand a home-relative path on Unix and Windows.
fn expand_home(path: &str) -> PathBuf {
    let relative = path.strip_prefix("~/").or_else(|| path.strip_prefix("~\\"));
    let home = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE"));
    match (relative, home) {
        (Some(rest), Some(home)) => PathBuf::from(home).join(rest),
        _ => PathBuf::from(path),
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
        p.song.tracks[1].mute = true;
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
        assert!(has_pitch(&p.song.tracks[1].notes[0].pitch, "D2"));
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
        p.song.tracks[2].mute = true;
        let json = serde_json::to_string(&p).unwrap();
        let back: Project = serde_json::from_str(&json).unwrap();
        assert!(back.cell(&back.rows[0].clone(), 0));
        assert_eq!(back.rows[4].pitch, "E2");
        assert!(back.song.tracks[2].mute);
        // The embedded song carries the current engine pin.
        assert_eq!(back.song.engine, p.song.engine);
    }
    #[test]
    fn pattern_mix_matches_the_recorded_baseline() {
        let mut project = Project::new();
        for (row, step) in [
            (0, 0),
            (0, 8),
            (1, 4),
            (2, 2),
            (2, 6),
            (4, 0),
            (4, 8),
            (6, 4),
        ] {
            project.toggle(row, step);
        }
        project.set_row_pitch(4, "Eb2");
        project.song.tracks[1].gain = 0.7;
        project.song.tracks[1].pan = -0.25;
        project.song.tracks[2].mute = true;
        let doc = project.loop_doc().unwrap().unwrap();
        let (left, right) = tono_core::render::render_product(&doc).into_stereo();
        let mut hash = 0xCBF2_9CE4_8422_2325u64;
        for x in left.iter().chain(&right) {
            for byte in x.to_bits().to_le_bytes() {
                hash = (hash ^ byte as u64).wrapping_mul(0x0000_0100_0000_01B3);
            }
        }
        assert_eq!(left.len(), 88200);
        assert_eq!(hash, 0xa15b_c9cf_9c9e_1793);
    }

    #[test]
    fn rejected_projects_are_replaced_in_memory_and_on_disk() {
        let folder = tempfile::tempdir().unwrap();
        let path = folder.path().join("project.json");
        let mut invalid = vec![b"not JSON".to_vec(), vec![0xff, 0xfe]];
        for (field, value) in [("engine", 0), ("version", 1)] {
            let mut saved = serde_json::to_value(Project::new()).unwrap();
            saved["song"][field] = serde_json::json!(value);
            invalid.push(serde_json::to_vec(&saved).unwrap());
        }
        let mut saved = serde_json::to_value(Project::new()).unwrap();
        saved["muted"] = serde_json::json!(["bass"]);
        invalid.push(serde_json::to_vec(&saved).unwrap());
        let mut saved = serde_json::to_value(Project::new()).unwrap();
        saved["bars"] = serde_json::json!(u32::MAX);
        invalid.push(serde_json::to_vec(&saved).unwrap());
        let mut saved = serde_json::to_value(Project::new()).unwrap();
        saved["rows"][0]["track"] = serde_json::json!("missing");
        invalid.push(serde_json::to_vec(&saved).unwrap());
        for bytes in invalid {
            let mut station = Station::new();
            station.project.toggle(0, 0);
            std::fs::write(&path, bytes).unwrap();
            let message = station.load(path.to_str().unwrap()).unwrap().unwrap();
            assert!(message.contains("fresh current project"), "{message}");
            assert_eq!(station.project.steps(), 16);
            assert!(station.project.loop_doc().unwrap().is_none());
            let stored: Project = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
            stored.validate().unwrap();
            station.load(path.to_str().unwrap()).unwrap();
        }
    }

    #[test]
    fn file_failures_preserve_the_working_project_and_saved_file() {
        let folder = tempfile::tempdir().unwrap();
        let path = folder.path().join("project.json");
        let mut station = Station::new();
        station.project.toggle(0, 0);
        station.project.song.tracks[1].mute = true;
        station.save(path.to_str().unwrap()).unwrap();
        let saved = std::fs::read(&path).unwrap();
        assert!(
            station
                .load(folder.path().join("missing.json").to_str().unwrap())
                .is_err()
        );
        assert!(station.project.cell(&station.project.rows[0], 0));
        assert!(station.save(folder.path().to_str().unwrap()).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), saved);
        station.project.song.bpm = 0.0;
        assert!(station.save(path.to_str().unwrap()).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), saved);
        station.project.song.bpm = 120.0;
        station.project.toggle(0, 4);
        station.save(path.to_str().unwrap()).unwrap();
        let mut restored = Station::new();
        restored.load(path.to_str().unwrap()).unwrap();
        assert!(restored.project.cell(&restored.project.rows[0], 4));
        assert!(restored.project.song.tracks[1].mute);
    }

    #[test]
    fn grid_edits_cannot_create_out_of_range_notes_or_alias_lanes() {
        let mut project = Project::new();
        project.toggle(0, project.steps());
        assert!(project.song.tracks[0].notes.is_empty());
        assert!(!project.set_row_pitch(4, "G1"));
        assert_eq!(project.rows[4].pitch, "C2");
        project.validate().unwrap();
    }
}
