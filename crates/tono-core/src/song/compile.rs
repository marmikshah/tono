//! Compiling a [`Song`](super::Song) to a deterministic [`SoundDoc`] — the
//! `tracks` root of `seq` tracks. Length/duration math lives here too, and
//! [`Song::compile`] — the full validation + lowering entry point that
//! returns an immutable [`Program`] (ADR 0003).

use super::{Song, SongError, SongTrack};
use crate::diag::{CompileError, Diagnostic};
use crate::dsl::{Node, SeqNote, SoundDoc, Track};
use crate::ids::TrackId;
use crate::program::{PROGRAM_VERSION, Program, ProgramMeta, TrackMeta, blocker_warnings};
use crate::units::Beat;

/// What a compiled [`Program`] will be used for. Offline compilation preserves
/// streaming blockers as warnings; runtime compilation promotes them to errors
/// so a target that requires native streaming cannot silently fall back to a
/// full pre-rendered buffer.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CompileTarget {
    /// Offline rendering (mix, ranges, stems).
    #[default]
    Offline,
    /// Real-time playback through the runtime engine.
    Runtime,
}

/// Knobs for [`Song::compile`]. `Default`: the document's own sample rate
/// (44 100 Hz), offline target.
#[derive(Debug, Clone, Default)]
pub struct CompileOptions {
    /// Stamp the resolved document with this sample rate (None keeps the
    /// document default, 44 100 Hz).
    pub sample_rate: Option<u32>,
    /// What the program will be used for.
    pub target: CompileTarget,
}

/// The reverb room size of a track's send (one shared, musical room).
const SEND_ROOM: f32 = 0.6;
/// Mix at full send — half wet keeps the dry signal audible under it.
const SEND_MIX_MAX: f32 = 0.5;

/// Where a note stops sounding, in steps. Zero-length notes still occupy one
/// step (the same floor the seq renderer applies). Saturating: a pathological
/// step/len wraps in release (and panics in debug) for no benefit — the seq
/// renderer already caps notes at the render window.
pub(crate) fn note_end(n: &SeqNote) -> u32 {
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
        let from_notes = if self.plain_meter() {
            self.tracks
                .iter()
                .flat_map(|t| t.notes.iter())
                .map(|n| note_end(n).div_ceil(steps_per_bar))
                .max()
                .unwrap_or(0)
        } else {
            self.tracks
                .iter()
                .flat_map(|t| t.notes.iter())
                .map(|n| {
                    let beat = Beat::new(note_end(n) as i64, self.steps_per_beat.max(1));
                    self.bar_count_at_beat(beat)
                })
                .max()
                .unwrap_or(0)
        };
        from_patterns.max(from_notes)
    }

    /// Bars elapsed at `beat` under the meter map — the shared walk
    /// ([`crate::units::bar_count_at_beat`]).
    fn bar_count_at_beat(&self, beat: Beat) -> u32 {
        crate::units::bar_count_at_beat(&self.meter_map, self.beats_per_bar, self.pickup, beat)
    }

    /// Steps per bar, degenerate (zero) fields floored to 1 — the one formula
    /// [`length_bars`](Self::length_bars) and [`to_doc`](Self::to_doc) share,
    /// so a deserialized song can't report a length its compile disagrees with.
    fn steps_per_bar(&self) -> u32 {
        self.beats_per_bar
            .max(1)
            .saturating_mul(self.steps_per_beat.max(1))
    }

    fn checked_steps_per_bar(&self) -> Result<u32, SongError> {
        self.beats_per_bar
            .max(1)
            .checked_mul(self.steps_per_beat.max(1))
            .ok_or_else(|| {
                SongError::Compile(format!(
                    "beats_per_bar ({}) × steps_per_beat ({}) exceeds the u32 song grid",
                    self.beats_per_bar, self.steps_per_beat
                ))
            })
    }

    /// Whether the meter is plain (`beats_per_bar`/4 throughout, no pickup) —
    /// the legacy placement path, byte-identical to before the maps existed.
    pub(crate) fn plain_meter(&self) -> bool {
        self.meter_map.is_empty() && self.pickup.is_none()
    }

    /// The exact beat `bar` starts at — the pickup plus the meter walk,
    /// segment-wise. The shared walk ([`crate::units::beat_at_bar`]) every
    /// tempo-aware path (compiler, transport) uses, so they never disagree.
    pub fn beat_at_bar(&self, bar: u32) -> Beat {
        crate::units::beat_at_bar(&self.meter_map, self.beats_per_bar, self.pickup, bar)
    }

    /// A beat position to a grid step, erroring when it lands between steps
    /// (the grid is the seq's; placements must sit on it).
    pub(crate) fn beat_to_step(&self, beat: Beat) -> Result<u32, SongError> {
        let spb = self.steps_per_beat.max(1) as i128;
        let num = beat.num as i128 * spb;
        if num % beat.den as i128 != 0 {
            return Err(SongError::Compile(format!(
                "a placement at beat {beat} doesn't land on the {}-steps-per-beat \
                 grid — change steps_per_beat or the meter map/pickup",
                self.steps_per_beat.max(1)
            )));
        }
        Ok(u32::try_from((num / beat.den as i128).max(0)).unwrap_or(u32::MAX))
    }

    /// Compile to a deterministic [`SoundDoc`] — a
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
        self.checked_steps_per_bar()?;

        let sec_per_step = 60.0 / (self.bpm.max(1.0) * self.steps_per_beat.max(1) as f32);
        let any_solo = self.tracks.iter().any(|t| t.solo);
        let mut end_step = 0u32;
        let mut doc_tracks = Vec::with_capacity(self.tracks.len());
        for t in &self.tracks {
            doc_tracks.push(self.compile_track(t, &mut end_step, any_solo)?);
        }

        let duration = if self.tempo_map.is_empty() {
            end_step as f32 * sec_per_step + 2.0 // tail for release/reverb
        } else {
            let end_beat = end_step as f64 / self.steps_per_beat.max(1) as f64;
            crate::dsl::tempo_map_seconds_at(&self.tempo_map, end_beat) as f32 + 2.0
        };
        let root = Node::Tracks {
            tracks: doc_tracks,
            master: self.master.clone(),
            buses: self.buses.clone(),
        };
        let json = serde_json::json!({
            "name": self.name,
            "duration": duration,
            "engine": self.engine,
            "version": self.version,
            "root": serde_json::to_value(&root).map_err(|e| SongError::Compile(e.to_string()))?,
        });
        let doc: crate::dsl::SoundDoc = serde_json::from_value(json)
            .map_err(|e| SongError::Compile(format!("song doc build: {e}")))?;
        Ok(doc)
    }

    /// Compile one song track to a mixer [`Track`]: merge its direct notes with
    /// its pattern placements, build the seq node, and wrap the reverb send.
    /// Extends `end_step` to the track's last note end. `any_solo` carries the
    /// song-level solo state: when any track is solo, every non-solo track is
    /// muted (a muted solo track stays muted).
    fn compile_track(
        &self,
        t: &SongTrack,
        end_step: &mut u32,
        any_solo: bool,
    ) -> Result<Track, SongError> {
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
            let offset = if self.plain_meter() {
                pl.bar.saturating_mul(steps_per_bar)
            } else {
                self.beat_to_step(self.beat_at_bar(pl.bar))?
            };
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
        // The song's tempo map applies to every track's seq (the grid is
        // shared); empty maps omit the field, so plain songs are unchanged.
        if !self.tempo_map.is_empty() {
            seq_json["tempo_map"] = serde_json::to_value(&self.tempo_map)
                .map_err(|e| SongError::Compile(e.to_string()))?;
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
            mute: t.mute || (any_solo && !t.solo),
            automation: t
                .automation
                .iter()
                .map(|lane| self.compile_lane(lane))
                .collect(),
            sidechain: None,
            bus: t.bus.clone(),
            sends: t.sends.clone(),
        })
    }

    /// A song lane (beats) to a document lane (seconds): through the tempo
    /// map when the song has one, else the constant bpm.
    fn compile_lane(&self, lane: &super::SongLane) -> crate::dsl::AutoLane {
        crate::dsl::AutoLane {
            target: lane.target,
            curve: lane.curve,
            points: lane
                .points
                .iter()
                .map(|p| crate::dsl::AutoPoint {
                    t: self.seconds_at_beat(p.at),
                    v: p.v,
                })
                .collect(),
        }
    }

    /// Seconds at a beat position on the song grid (the lane conversion).
    fn seconds_at_beat(&self, beat: f32) -> f32 {
        if self.tempo_map.is_empty() {
            beat * 60.0 / self.bpm.max(1.0)
        } else {
            crate::dsl::tempo_map_seconds_at(&self.tempo_map, beat as f64) as f32
        }
    }
}

impl Song {
    /// Compile the song into an immutable, hashed [`Program`] — the central
    /// validation + lowering entry point (ADR 0003). Validation collects
    /// every problem in one pass (unknown references, a document that fails
    /// validation); the returned artifact carries the resolved document,
    /// musical metadata, bounded resource estimates, streaming-coverage
    /// warnings, and a canonical semantic hash that a Python-authored
    /// equivalent song reproduces exactly.
    ///
    /// This API is **stable** — frozen at 1.10.0-rc.1
    /// (docs/api-tiers.md).
    ///
    /// ```
    /// use tono_core::song::{CompileOptions, Song, note};
    /// use tono_core::dsl::{Adsr, SeqWave};
    ///
    /// let amp = Adsr { a: 0.005, d: 0.1, s: 0.8, r: 0.2, punch: 0.0 };
    /// let mut song = Song::new("demo", 120.0);
    /// song.add_track("bass", SeqWave::Bass, amp);
    /// song.add_pattern("riff", 1, vec![note(0, 4, "C2")]);
    /// song.arrange("bass", "riff", 0);
    /// let program = song.compile(&CompileOptions::default()).unwrap();
    /// assert!(!program.render_mono().is_empty());
    /// ```
    pub fn compile(&self, opts: &CompileOptions) -> Result<Program, CompileError> {
        // One pass, every problem collected — the author fixes one compile,
        // not a drip-feed of first errors.
        let mut diags = CompileError::default();
        if self.tracks.is_empty() {
            diags.push(Diagnostic::from(&SongError::Empty));
        }
        for (i, pl) in self.arrangement.iter().enumerate() {
            if !self.tracks.iter().any(|t| t.name == pl.track) {
                let mut d = Diagnostic::from(&SongError::UnknownTrack(pl.track.clone()));
                d.path = format!("arrangement[{i}].track");
                diags.push(d);
            }
            if !self.patterns.iter().any(|p| p.name == pl.pattern) {
                let mut d = Diagnostic::from(&SongError::UnknownPattern(pl.pattern.clone()));
                d.path = format!("arrangement[{i}].pattern");
                diags.push(d);
            }
        }
        if diags.has_errors() {
            return Err(diags);
        }

        self.validate_maps(&mut diags);
        if diags.has_errors() {
            return Err(diags);
        }

        let mut doc = match self.to_doc() {
            Ok(doc) => doc,
            Err(e) => {
                diags.push(Diagnostic::from(&e));
                return Err(diags);
            }
        };
        if let Some(rate) = opts.sample_rate {
            doc.sample_rate = rate;
        }
        if let Some(seed) = self.seed {
            doc.seed = seed;
        }
        if let Err(e) = doc.validate() {
            diags.push(
                Diagnostic::error("T2000", "doc", e.to_string())
                    .with_remediation("fix the flagged document field and recompile"),
            );
            return Err(diags);
        }

        let warnings = blocker_warnings(&doc);
        if opts.target == CompileTarget::Runtime && !warnings.is_empty() {
            for mut blocker in warnings {
                blocker.severity = crate::diag::Severity::Error;
                diags.push(blocker);
            }
            return Err(diags);
        }
        let meta = self.program_meta(&doc);
        let estimates = super::estimate::program_estimates(&doc);
        let mut program = Program {
            program_version: PROGRAM_VERSION,
            schema_version: doc.version,
            engine_version: doc.engine,
            hash: 0,
            target: opts.target,
            doc,
            meta,
            estimates,
            warnings,
        };
        program.hash = program.computed_hash();
        Ok(program)
    }

    /// The [`ProgramMeta`] of the resolved document: the musical facts a
    /// transport needs, captured at compile time.
    fn program_meta(&self, doc: &SoundDoc) -> ProgramMeta {
        let mut sections = self.sections.clone();
        sections.sort_by_key(|s| s.bar);
        let mut markers = self.markers.clone();
        markers.sort_by_key(|m| m.at);
        ProgramMeta {
            name: doc.name.clone(),
            tempo_bpm: self.bpm.max(1.0),
            beats_per_bar: self.beats_per_bar.max(1),
            steps_per_beat: self.steps_per_beat.max(1),
            tempo_map: self.tempo_map.clone(),
            meter_map: self.meter_map.clone(),
            pickup: self.pickup,
            sections,
            markers,
            length_bars: self.length_bars(),
            duration_secs: doc.duration,
            duration_frames: super::estimate::duration_frames(doc),
            sample_rate: doc.sample_rate,
            tracks: self
                .tracks
                .iter()
                .enumerate()
                .map(|(i, t)| TrackMeta {
                    id: TrackId::from(i as u64 + 1),
                    name: t.name.clone(),
                    wave: t.wave,
                    notes: super::estimate::track_note_count(doc, i),
                    mute: t.mute,
                    solo: t.solo,
                })
                .collect(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dsl::{Adsr, SeqWave};
    use crate::song::note;
    use crate::units::MeterPoint;

    fn amp() -> Adsr {
        Adsr {
            a: 0.005,
            d: 0.1,
            s: 0.8,
            r: 0.2,
            punch: 0.0,
        }
    }

    fn demo_song() -> Song {
        let mut song = Song::new("demo", 120.0);
        song.add_track("bass", SeqWave::Bass, amp());
        song.add_track("keys", SeqWave::Epiano, amp());
        song.add_pattern("riff", 1, vec![note(0, 4, "C2"), note(8, 4, "G2")]);
        song.add_pattern("stab", 1, vec![note(4, 2, "C4")]);
        song.arrange("bass", "riff", 0);
        song.arrange("keys", "stab", 0);
        song
    }

    #[test]
    fn compile_collects_every_unknown_reference_in_one_pass() {
        let mut song = Song::new("s", 120.0);
        song.add_track("t", SeqWave::Sine, amp());
        song.arrange("nope", "ghost", 0);
        song.arrange("alsono", "ghost2", 1);
        let err = song.compile(&CompileOptions::default()).unwrap_err();
        let codes: Vec<_> = err.0.iter().map(|d| (d.code, d.path.as_str())).collect();
        assert_eq!(
            codes,
            vec![
                ("T1001", "arrangement[0].track"),
                ("T1002", "arrangement[0].pattern"),
                ("T1001", "arrangement[1].track"),
                ("T1002", "arrangement[1].pattern"),
            ],
            "every bad reference reported, with its exact path"
        );
    }

    #[test]
    fn compile_an_empty_song_is_t1000() {
        let song = Song::new("s", 120.0);
        let err = song.compile(&CompileOptions::default()).unwrap_err();
        assert_eq!(err.0.len(), 1);
        assert_eq!(err.0[0].code, "T1000");
    }

    #[test]
    fn an_overflowing_deserialized_grid_is_a_compile_error() {
        let mut song = demo_song();
        song.beats_per_bar = u32::MAX;
        song.steps_per_beat = u32::MAX;
        let err = song.compile(&CompileOptions::default()).unwrap_err();
        assert!(err.to_string().contains("exceeds the u32 song grid"));
    }

    #[test]
    fn compile_is_deterministic() {
        let a = demo_song().compile(&CompileOptions::default()).unwrap();
        let b = demo_song().compile(&CompileOptions::default()).unwrap();
        assert_eq!(a.hash, b.hash);
        assert_eq!(a.render_mono(), b.render_mono());
    }

    #[test]
    fn the_seed_stamps_the_doc_and_moves_the_hash() {
        let plain = demo_song().compile(&CompileOptions::default()).unwrap();
        assert_eq!(plain.doc.seed, 0);
        let seeded = demo_song()
            .with_seed(7)
            .compile(&CompileOptions::default())
            .unwrap();
        assert_eq!(seeded.doc.seed, 7);
        assert_ne!(plain.hash, seeded.hash, "the seed is part of the artifact");
        // And stays deterministic per seed.
        let again = demo_song()
            .with_seed(7)
            .compile(&CompileOptions::default())
            .unwrap();
        assert_eq!(seeded.hash, again.hash);
    }

    #[test]
    fn mute_and_solo_reach_the_mixer() {
        let mut song = demo_song();
        song.tracks[1].solo = true;
        let program = song.compile(&CompileOptions::default()).unwrap();
        let Node::Tracks { tracks, .. } = &program.doc.root else {
            panic!("tracks root");
        };
        assert!(tracks[0].mute, "the non-solo track is muted");
        assert!(!tracks[1].mute, "the solo track sounds");
        // A muted solo track stays muted.
        let mut song = demo_song();
        song.tracks[1].solo = true;
        song.tracks[1].mute = true;
        let program = song.compile(&CompileOptions::default()).unwrap();
        let Node::Tracks { tracks, .. } = &program.doc.root else {
            panic!("tracks root");
        };
        assert!(tracks[1].mute);
    }

    #[test]
    fn compile_option_sample_rate_stamps_the_program() {
        let program = demo_song()
            .compile(&CompileOptions {
                sample_rate: Some(48_000),
                ..CompileOptions::default()
            })
            .unwrap();
        assert_eq!(program.doc.sample_rate, 48_000);
        assert_eq!(program.meta.sample_rate, 48_000);
    }

    #[test]
    fn a_streamable_tracks_root_compiles_without_warnings() {
        // A plain compiled song is a mixer whose parts all stream
        // (built-in seq waves, no master chain): no TracksRoot warning, and
        // is_streamable follows.
        let program = demo_song().compile(&CompileOptions::default()).unwrap();
        assert!(
            program.is_streamable(),
            "a plain compiled song streams natively now: {:?}",
            program.warnings
        );
        assert!(program.warnings.is_empty());
    }

    #[test]
    fn a_tracks_root_with_an_unstreamable_part_still_warns() {
        // A master-chain convolve can't stream — the program keeps the
        // warning (and the Player fallback), now naming the failing part.
        let mut song = demo_song();
        song.master.push(
            serde_json::from_value(
                serde_json::json!({ "type": "convolve", "decay": 0.6, "mix": 0.4 }),
            )
            .unwrap(),
        );
        let program = song.compile(&CompileOptions::default()).unwrap();
        assert!(!program.is_streamable());
        assert!(
            program
                .warnings
                .iter()
                .any(|d| d.code == "T1508" && d.message.contains("the master chain")),
            "the master-chain blocker is a warning with its context: {:?}",
            program.warnings
        );
        assert!(
            program
                .warnings
                .iter()
                .all(|d| d.severity == crate::diag::Severity::Warning),
            "warnings never fail a compile"
        );
    }

    #[test]
    fn runtime_target_rejects_streaming_blockers() {
        let mut song = demo_song();
        song.master.push(
            serde_json::from_value(
                serde_json::json!({ "type": "convolve", "decay": 0.6, "mix": 0.4 }),
            )
            .unwrap(),
        );
        let err = song
            .compile(&CompileOptions {
                target: CompileTarget::Runtime,
                ..CompileOptions::default()
            })
            .unwrap_err();
        assert!(err.has_errors());
        assert!(
            err.0
                .iter()
                .any(|d| d.code == "T1508" && d.severity == crate::diag::Severity::Error)
        );
    }

    #[test]
    fn an_invalid_resolved_doc_is_t2000() {
        // A sampler track without a SoundFont path resolves but fails
        // document validation at compile.
        let mut song = Song::new("s", 120.0);
        song.add_track("keys", SeqWave::Sampler, amp());
        song.tracks[0].notes.push(note(0, 4, "C4"));
        let err = song.compile(&CompileOptions::default()).unwrap_err();
        assert_eq!(err.0.len(), 1);
        assert_eq!(err.0[0].code, "T2000");
        assert_eq!(err.0[0].path, "doc");
    }

    #[test]
    fn meta_preserves_the_musical_facts() {
        let program = demo_song().compile(&CompileOptions::default()).unwrap();
        assert_eq!(program.meta.name, "demo");
        assert_eq!(program.meta.tempo_bpm, 120.0);
        assert_eq!(program.meta.length_bars, 1);
        assert_eq!(program.meta.tracks.len(), 2);
        assert_eq!(program.meta.tracks[0].id.get(), 1);
        assert_eq!(program.meta.tracks[1].id.get(), 2);
        assert_eq!(program.meta.tracks[0].name, "bass");
        assert_eq!(program.meta.tracks[0].notes, 2);
        assert_eq!(
            program.meta.duration_frames,
            (program.doc.duration * program.doc.sample_rate as f32).round() as u64
        );
    }

    #[test]
    fn estimates_count_events_and_peak_voices() {
        let mut song = Song::new("s", 120.0);
        song.add_track("chords", SeqWave::Organ, amp());
        // Three overlapping notes (a chord) plus a later single.
        song.tracks[0].notes.push(note(0, 8, "C4"));
        song.tracks[0].notes.push(note(0, 8, "E4"));
        song.tracks[0].notes.push(note(0, 8, "G4"));
        song.tracks[0].notes.push(note(8, 4, "A4"));
        let program = song.compile(&CompileOptions::default()).unwrap();
        assert_eq!(program.estimates.events, 4);
        assert_eq!(program.estimates.peak_voices, 3, "the chord is 3 voices");
    }

    #[test]
    fn program_renders_stereo_matching_the_doc() {
        let program = demo_song().compile(&CompileOptions::default()).unwrap();
        let (l, r) = program.render_stereo();
        let product = crate::render::render_product(&program.doc);
        let (el, er) = product.stereo.unwrap();
        assert_eq!(l, el);
        assert_eq!(r, er);
    }

    #[test]
    fn tempo_map_walk_is_segment_exact() {
        // 120 BPM for 4 beats, then 240: beat 8 lands at 2.0 + 1.0 = 3.0 s.
        let map = vec![
            crate::dsl::TempoPoint {
                at: Beat::zero(),
                bpm: 120.0,
            },
            crate::dsl::TempoPoint {
                at: Beat::from_int(4),
                bpm: 240.0,
            },
        ];
        assert_eq!(crate::dsl::tempo_map_seconds_at(&map, 0.0), 0.0);
        assert_eq!(crate::dsl::tempo_map_seconds_at(&map, 4.0), 2.0);
        assert_eq!(crate::dsl::tempo_map_seconds_at(&map, 8.0), 3.0);
        assert_eq!(crate::dsl::tempo_map_bpm_at(&map, 3.999), 120.0);
        assert_eq!(crate::dsl::tempo_map_bpm_at(&map, 4.0), 240.0);
    }

    fn tempo_mapped_song() -> Song {
        let mut song = demo_song();
        song.tempo_map = vec![
            crate::dsl::TempoPoint {
                at: Beat::zero(),
                bpm: 120.0,
            },
            crate::dsl::TempoPoint {
                at: Beat::from_int(4),
                bpm: 240.0,
            },
        ];
        song
    }

    #[test]
    fn tempo_map_reaches_the_seq_and_the_meta() {
        let program = tempo_mapped_song()
            .compile(&CompileOptions {
                sample_rate: Some(48_000),
                ..CompileOptions::default()
            })
            .unwrap();
        let Node::Tracks { tracks, .. } = &program.doc.root else {
            panic!("tracks root");
        };
        let Node::Seq { tempo_map, .. } = &tracks[0].node else {
            panic!("seq track");
        };
        assert_eq!(tempo_map.len(), 2, "the seq carries the map");
        assert_eq!(program.meta.tempo_map.len(), 2, "the meta preserves it");
        // The last note ends at beat 3 (step 12 + len 4 at 4 spb): 1.5 s at
        // 120 BPM, + 2 s tail.
        assert!(
            (program.doc.duration - 3.5).abs() < 1e-4,
            "{}",
            program.doc.duration
        );
    }

    #[test]
    fn tempo_map_places_notes_on_exact_frames() {
        // One note at beat 0 and one at beat 8 (step 32): with 120 → 240 at
        // beat 4, the second starts at exactly 3.0 s = frame 144 000 at 48 kHz.
        let json = r#"{ "name": "mapped", "duration": 4.0, "version": 2, "engine": 5,
            "sample_rate": 48000,
            "root": { "type": "seq", "bpm": 120, "wave": "sawtooth",
                "tempo_map": [ { "at": { "num": 0, "den": 1 }, "bpm": 120 },
                               { "at": { "num": 4, "den": 1 }, "bpm": 240 } ],
                "env": { "a": 0.0, "d": 0.0, "s": 1.0, "r": 0.01 },
                "notes": [ { "step": 0, "len": 4, "pitch": "A4" },
                           { "step": 32, "len": 4, "pitch": "A4" } ] } }"#;
        let doc: SoundDoc = serde_json::from_str(json).unwrap();
        doc.validate().unwrap();
        let out = crate::render::render(&doc);
        // The saw starts at −1, so the onset frame is the first nonzero sample.
        let onsets: Vec<usize> = {
            let mut marks = Vec::new();
            let mut silent = true;
            for (i, &s) in out.iter().enumerate() {
                if silent && s != 0.0 {
                    marks.push(i);
                    silent = false;
                } else if !silent && s == 0.0 {
                    silent = true;
                }
            }
            marks
        };
        assert_eq!(onsets.len(), 2, "two notes: {onsets:?}");
        assert_eq!(
            onsets[1] - onsets[0],
            144_000,
            "the note past the tempo change lands exactly 3.0 s later: {onsets:?}"
        );
    }

    #[test]
    fn a_tempo_mapped_seq_streams_byte_identically() {
        let json = r#"{ "name": "mapped", "duration": 1.0, "version": 2, "engine": 5,
            "root": { "type": "seq", "bpm": 120, "wave": "square",
                "tempo_map": [ { "at": { "num": 0, "den": 1 }, "bpm": 120 },
                               { "at": { "num": 2, "den": 1 }, "bpm": 90 } ],
                "env": { "a": 0.005, "d": 0.05, "s": 0.6, "r": 0.05 },
                "notes": [ { "step": 0, "len": 2, "pitch": "C4" },
                           { "step": 6, "len": 2, "pitch": "E4" },
                           { "step": 12, "len": 2, "pitch": "G4" } ] } }"#;
        let doc: SoundDoc = serde_json::from_str(json).unwrap();
        doc.validate().unwrap();
        crate::streaming::tests::assert_byte_identical(&doc);
    }

    #[test]
    fn meter_map_and_pickup_place_bars_exactly() {
        // 6/8: bar 1 starts at beat 3 (step 12 at 4 spb).
        let mut song = Song::new("waltzish", 120.0);
        song.meter_map = vec![MeterPoint {
            bar: 0,
            numerator: 6,
            denominator: 8,
        }];
        song.add_track("t", SeqWave::Sine, amp());
        song.add_pattern("p", 1, vec![note(0, 1, "C4")]);
        song.arrange("t", "p", 1);
        assert_eq!(song.beat_at_bar(1), Beat::from_int(3));
        assert_eq!(song.beat_at_bar(2), Beat::from_int(6));
        let program = song.compile(&CompileOptions::default()).unwrap();
        let Node::Tracks { tracks, .. } = &program.doc.root else {
            panic!("tracks");
        };
        let Node::Seq { notes, .. } = &tracks[0].node else {
            panic!("seq");
        };
        assert_eq!(notes[0].step, 12, "bar 1 of 6/8 is step 12");
        // A one-beat pickup shifts bar 1 to step 4.
        song.pickup = Some(Beat::from_int(1));
        let program = song.compile(&CompileOptions::default()).unwrap();
        let Node::Tracks { tracks, .. } = &program.doc.root else {
            panic!("tracks");
        };
        let Node::Seq { notes, .. } = &tracks[0].node else {
            panic!("seq");
        };
        assert_eq!(notes[0].step, 4, "bar 1 follows the one-beat pickup");
    }

    #[test]
    fn off_grid_placements_are_t1005() {
        let mut song = Song::new("s", 120.0);
        song.pickup = Some(Beat::new(1, 3)); // a third of a beat at 4 spb
        song.add_track("t", SeqWave::Sine, amp());
        song.add_pattern("p", 1, vec![note(0, 1, "C4")]);
        song.arrange("t", "p", 1);
        let err = song.compile(&CompileOptions::default()).unwrap_err();
        assert!(
            err.0
                .iter()
                .any(|d| d.code == "T1005" && d.path == "arrangement[0].bar"),
            "{:?}",
            err.0
        );
    }

    #[test]
    fn map_and_section_violations_have_their_codes() {
        let mut song = demo_song();
        song.tempo_map = vec![crate::dsl::TempoPoint {
            at: Beat::from_int(2),
            bpm: 140.0,
        }];
        let err = song.compile(&CompileOptions::default()).unwrap_err();
        assert!(err.0.iter().any(|d| d.code == "T1003"), "{:?}", err.0);

        let mut song = demo_song();
        song.meter_map = vec![MeterPoint {
            bar: 0,
            numerator: 3,
            denominator: 5,
        }];
        let err = song.compile(&CompileOptions::default()).unwrap_err();
        assert!(err.0.iter().any(|d| d.code == "T1004"), "{:?}", err.0);

        let mut song = demo_song();
        song.sections.push(crate::song::Section {
            name: String::new(),
            bar: 0,
            bars: 4,
        });
        let err = song.compile(&CompileOptions::default()).unwrap_err();
        assert!(err.0.iter().any(|d| d.code == "T1006"), "{:?}", err.0);
    }

    #[test]
    fn sections_and_markers_reach_the_meta_sorted() {
        let mut song = demo_song();
        song.sections.push(crate::song::Section {
            name: "chorus".into(),
            bar: 4,
            bars: 4,
        });
        song.sections.push(crate::song::Section {
            name: "verse".into(),
            bar: 0,
            bars: 4,
        });
        song.markers.push(crate::song::Marker {
            name: "drop".into(),
            at: Beat::from_int(16),
        });
        let program = song.compile(&CompileOptions::default()).unwrap();
        assert_eq!(program.meta.sections[0].name, "verse");
        assert_eq!(program.meta.sections[1].name, "chorus");
        assert_eq!(program.meta.markers[0].name, "drop");
        // And they survive the bundle round-trip.
        let loaded = crate::program::Program::from_json(&program.to_json()).unwrap();
        assert_eq!(loaded.meta.sections.len(), 2);
    }

    #[test]
    fn length_bars_respects_the_meter_map() {
        // 6/8 (3 beats a bar): a note ending at beat 6 closes bar 2.
        let mut song = Song::new("s", 120.0);
        song.meter_map = vec![MeterPoint {
            bar: 0,
            numerator: 6,
            denominator: 8,
        }];
        song.add_track("t", SeqWave::Sine, amp());
        song.tracks[0].notes.push(note(0, 24, "C4")); // 24 steps = 6 beats
        assert_eq!(song.length_bars(), 2);
        // In 4/4 the same note reaches only bar 2's start too (6 beats = 1.5 bars → 2).
        song.meter_map.clear();
        assert_eq!(song.length_bars(), 2);
    }

    #[test]
    fn automation_compiles_beats_to_seconds() {
        let mut song = Song::new("s", 120.0);
        song.add_track("t", SeqWave::Sine, amp());
        song.tracks[0].notes.push(note(0, 4, "C4"));
        song.tracks[0].automation.push(crate::song::SongLane {
            target: crate::dsl::AutoTarget::Gain,
            curve: crate::dsl::AutoCurve::Step,
            points: vec![
                crate::song::SongPoint { at: 0.0, v: 0.2 },
                crate::song::SongPoint { at: 2.0, v: 0.8 },
            ],
        });
        let program = song.compile(&CompileOptions::default()).unwrap();
        let Node::Tracks { tracks, .. } = &program.doc.root else {
            panic!("tracks root");
        };
        assert_eq!(tracks[0].automation.len(), 1);
        assert_eq!(tracks[0].automation[0].curve, crate::dsl::AutoCurve::Step);
        assert_eq!(
            tracks[0].automation[0].points[1].t, 1.0,
            "beat 2 at 120 BPM is 1.0 s"
        );

        // Through a tempo map: 120 → 60 at beat 2, so beat 4 is 1 + 2 = 3 s.
        song.tempo_map = vec![
            crate::dsl::TempoPoint {
                at: Beat::zero(),
                bpm: 120.0,
            },
            crate::dsl::TempoPoint {
                at: Beat::from_int(2),
                bpm: 60.0,
            },
        ];
        song.tracks[0].automation[0].points[1].at = 4.0;
        let program = song.compile(&CompileOptions::default()).unwrap();
        let Node::Tracks { tracks, .. } = &program.doc.root else {
            panic!("tracks root");
        };
        assert_eq!(
            tracks[0].automation[0].points[1].t, 3.0,
            "the lane crosses the tempo map segment-wise"
        );
    }

    #[test]
    fn buses_and_sends_pass_through_to_the_document() {
        let mut song = demo_song();
        song.buses.push(crate::dsl::Bus {
            id: "verb".into(),
            gain: 0.8,
            effects: vec![Node::Reverb {
                room: 0.6,
                mix: 0.4,
            }],
        });
        song.tracks[1].bus = Some("verb".into());
        song.tracks[0].sends.push(crate::dsl::Send {
            bus: "verb".into(),
            amount: 0.3,
        });
        let program = song.compile(&CompileOptions::default()).unwrap();
        let Node::Tracks { tracks, buses, .. } = &program.doc.root else {
            panic!("tracks root");
        };
        assert_eq!(buses.len(), 1);
        assert_eq!(buses[0].id, "verb");
        assert_eq!(tracks[1].bus.as_deref(), Some("verb"));
        assert_eq!(tracks[0].sends.len(), 1);
        assert_eq!(tracks[0].sends[0].bus, "verb");
        assert!(program.doc.validate().is_ok(), "the wired mix validates");
        // And the mix renders (the send leaves a reverb tail).
        assert!(!program.render_mono().is_empty());
    }
}
