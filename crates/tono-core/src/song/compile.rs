//! Compiling a [`Song`](super::Song) to a deterministic [`SoundDoc`] — the
//! `tracks` root of `seq` tracks. Length/duration math lives here too.

use super::{Song, SongError, SongTrack};
use crate::dsl::{ENGINE_VERSION, Node, SeqNote, Track};

/// The reverb room size of a track's send (one shared, musical room).
const SEND_ROOM: f32 = 0.6;
/// Mix at full send — half wet keeps the dry signal audible under it.
const SEND_MIX_MAX: f32 = 0.5;

/// Where a note stops sounding, in steps. Zero-length notes still occupy one
/// step (the same floor the seq renderer applies). Saturating: a pathological
/// step/len wraps in release (and panics in debug) for no benefit — the seq
/// renderer already caps notes at the render window.
fn note_end(n: &SeqNote) -> u32 {
    n.step.saturating_add(n.len.max(1))
}

impl Song {
    /// The song's length in bars: the end of its last-ending pattern or of the
    /// last note written directly onto a track (the fluent [`Song::add`] path).
    pub fn length_bars(&self) -> u32 {
        let steps_per_bar = self.steps_per_bar();
        let from_patterns = self
            .arrangement
            .iter()
            .map(|pl| {
                let bars = self
                    .patterns
                    .iter()
                    .find(|p| p.name == pl.pattern)
                    .map(|p| p.bars)
                    .unwrap_or(0);
                pl.bar.saturating_add(bars)
            })
            .max()
            .unwrap_or(0);
        let from_notes = self
            .tracks
            .iter()
            .flat_map(|t| t.notes.iter())
            .map(|n| note_end(n).div_ceil(steps_per_bar))
            .max()
            .unwrap_or(0);
        from_patterns.max(from_notes)
    }

    /// Steps per bar, degenerate (zero) fields floored to 1 — the one formula
    /// [`length_bars`](Self::length_bars) and [`to_doc`](Self::to_doc) share,
    /// so a deserialized song can't report a length its compile disagrees with.
    fn steps_per_bar(&self) -> u32 {
        self.beats_per_bar.max(1) * self.steps_per_beat.max(1)
    }

    /// Compile to a deterministic [`SoundDoc`](crate::dsl::SoundDoc) — a
    /// `tracks` root of `seq` tracks. Errors if the song is empty or an
    /// arrangement references a missing track or pattern.
    pub fn to_doc(&self) -> Result<crate::dsl::SoundDoc, SongError> {
        if self.tracks.is_empty() {
            return Err(SongError::Empty);
        }
        for pl in &self.arrangement {
            if !self.tracks.iter().any(|t| t.name == pl.track) {
                return Err(SongError::UnknownTrack(pl.track.clone()));
            }
            if !self.patterns.iter().any(|p| p.name == pl.pattern) {
                return Err(SongError::UnknownPattern(pl.pattern.clone()));
            }
        }

        let sec_per_step = 60.0 / (self.bpm.max(1.0) * self.steps_per_beat.max(1) as f32);
        let mut end_step = 0u32;
        let mut doc_tracks = Vec::with_capacity(self.tracks.len());
        for t in &self.tracks {
            doc_tracks.push(self.compile_track(t, &mut end_step)?);
        }

        let duration = end_step as f32 * sec_per_step + 2.0; // tail for release/reverb
        let root = Node::Tracks {
            tracks: doc_tracks,
            master: self.master.clone(),
        };
        // The song's pinned engine/version win over the current ones, so a
        // saved project replays byte-identically across kernel upgrades.
        // Older saves without the pins keep their historical behavior: the
        // current engine, v1 schema semantics.
        let mut json = serde_json::json!({
            "name": self.name,
            "duration": duration,
            "engine": self.engine.unwrap_or(ENGINE_VERSION),
            "root": serde_json::to_value(&root).map_err(|e| SongError::Compile(e.to_string()))?,
        });
        if let Some(v) = self.version {
            json["version"] = serde_json::json!(v);
        }
        let doc: crate::dsl::SoundDoc = serde_json::from_value(json)
            .map_err(|e| SongError::Compile(format!("song doc build: {e}")))?;
        Ok(doc)
    }

    /// Compile one song track to a mixer [`Track`]: merge its direct notes with
    /// its pattern placements, build the seq node, and wrap the reverb send.
    /// Extends `end_step` to the track's last note end.
    fn compile_track(&self, t: &SongTrack, end_step: &mut u32) -> Result<Track, SongError> {
        let steps_per_bar = self.steps_per_bar();
        let mut notes: Vec<SeqNote> = t.notes.clone();
        for n in &notes {
            *end_step = (*end_step).max(note_end(n));
        }
        for pl in self.arrangement.iter().filter(|p| p.track == t.name) {
            let pat = self
                .patterns
                .iter()
                .find(|p| p.name == pl.pattern)
                .expect("pattern existence checked above");
            let offset = pl.bar.saturating_mul(steps_per_bar);
            for n in &pat.notes {
                let placed = SeqNote {
                    step: n.step.saturating_add(offset),
                    len: n.len,
                    pitch: n.pitch.clone(),
                    gain: n.gain,
                };
                *end_step = (*end_step).max(note_end(&placed));
                notes.push(placed);
            }
        }
        notes.sort_by_key(|n| n.step);

        // Build the seq node via serde so the seq-only fields (duty, fm_*,
        // pluck_decay) take the engine's own defaults — then merge the whole
        // VoiceParams struct over it. Field names match the seq node's keys
        // one-for-one, so every set knob flows through and a newly added voice
        // param can never be silently dropped here.
        let mut seq_json = serde_json::json!({
            "type": "seq",
            // bpm/steps_per_beat are clamped exactly like to_doc's duration
            // math — degenerate values would otherwise place notes beyond the
            // computed duration (silently dropping them) or build an invalid seq.
            "bpm": self.bpm.max(1.0),
            "steps_per_beat": self.steps_per_beat.max(1),
            "wave": serde_json::to_value(t.wave).map_err(|e| SongError::Compile(e.to_string()))?,
            "env": serde_json::to_value(t.env).map_err(|e| SongError::Compile(e.to_string()))?,
            "swing": t.swing.unwrap_or(self.swing),
            "humanize": t.humanize.unwrap_or(self.humanize),
            "sf2": t.sf2,
            "sf2_preset": t.sf2_preset,
            "sf2_bank": t.sf2_bank,
            "notes": serde_json::to_value(&notes).map_err(|e| SongError::Compile(e.to_string()))?,
        });
        if let serde_json::Value::Object(voice) =
            serde_json::to_value(t.voice).map_err(|e| SongError::Compile(e.to_string()))?
        {
            for (key, val) in voice {
                if !val.is_null() {
                    seq_json[key] = val;
                }
            }
        }
        let seq: Node = serde_json::from_value(seq_json)
            .map_err(|e| SongError::Compile(format!("track '{}' seq build: {e}", t.name)))?;

        // A reverb send wraps the seq in a chain (dry when reverb == 0, so
        // the track is byte-identical without it).
        let node = if t.reverb > 0.0 {
            let rv = t.reverb.clamp(0.0, 1.0);
            Node::Chain {
                stages: vec![
                    seq,
                    Node::Reverb {
                        room: SEND_ROOM,
                        mix: SEND_MIX_MAX * rv,
                    },
                ],
            }
        } else {
            seq
        };
        Ok(Track {
            id: Some(t.name.clone()),
            node,
            pan: t.pan,
            gain: t.gain,
            at: 0.0,
            mute: false,
            automation: Vec::new(),
        })
    }
}
