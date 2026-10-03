//! Compiled songs: a resolved sound document, musical metadata, resource
//! bounds and a canonical integrity hash. Rendering and transport share the
//! document's format, sample rate and duration.

use serde::{Deserialize, Serialize};

use crate::diag::Diagnostic;
use crate::dsl::{SeqWave, SoundDoc};
use crate::render;
use crate::streaming::StreamGraph;

/// The current Program bundle format revision. A Program records the revision
/// it was compiled with; a loader accepts only this revision. Bumped
/// when the serialized shape (or its semantics) changes — independently of
/// the document schema and DSP engine revisions.
pub const PROGRAM_VERSION: u32 = 3;

/// A compiled song: validated, resolved, hashed. Built by
/// [`Song::compile`](crate::song::Song::compile); reloaded by
/// [`Program::from_json`].
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Program {
    /// The bundle revision (see [`PROGRAM_VERSION`]).
    pub program_version: u32,
    /// Canonical semantic hash over every serialized field except this hash.
    pub hash: u64,
    /// The target this program was compiled for (offline or runtime).
    #[serde(default)]
    pub target: crate::song::CompileTarget,
    /// The resolved document — renders through the exact same engine as
    /// everything else; nothing new in the render path.
    pub doc: SoundDoc,
    /// The musical metadata a transport (or a host deciding how far ahead to
    /// schedule) needs — preserved at compile time, not reconstructed.
    pub meta: ProgramMeta,
    /// Bounded resource estimates the runtime preallocates from (ADR 0005).
    pub estimates: ResourceEstimates,
}

/// The musical facts of a compiled song, resolved once at compile time.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProgramMeta {
    /// Tempo in beats per minute (clamped at compile, like the compiler's
    /// duration math: degenerate tempos floor at 1).
    pub tempo_bpm: f32,
    /// Time-signature numerator (4 = 4/4) — the default meter before any
    /// `meter_map` points.
    pub beats_per_bar: u32,
    /// Grid resolution (4 = sixteenth notes).
    pub steps_per_beat: u32,
    /// Tempo changes at exact beat positions (empty = constant `tempo_bpm`).
    #[serde(default)]
    pub tempo_map: Vec<crate::dsl::TempoPoint>,
    /// Time-signature changes by bar (empty = `beats_per_bar`/4 throughout).
    #[serde(default)]
    pub meter_map: Vec<crate::song::MeterPoint>,
    /// The pickup bar's length in beats, if any.
    #[serde(default)]
    pub pickup: Option<crate::units::Beat>,
    /// Named bar ranges, sorted by bar — the runtime's transition targets.
    #[serde(default)]
    pub sections: Vec<crate::song::Section>,
    /// Named beat points, sorted by position.
    #[serde(default)]
    pub markers: Vec<crate::song::Marker>,
    /// The song's length in bars (end of its last placement or direct note).
    pub length_bars: u32,
    /// One entry per track, in declaration order.
    pub tracks: Vec<TrackMeta>,
}

/// One track's identity and role in a compiled program.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TrackMeta {
    /// The track name (also the rendered layer id).
    pub name: String,
    /// The instrument voice.
    pub wave: SeqWave,
    /// How many notes the track plays (direct notes plus placements).
    pub notes: u32,
    /// Whether the track is muted in the mix.
    pub mute: bool,
    /// Whether the track is a solo track (every non-solo track is muted).
    pub solo: bool,
}

/// Bounded estimates of what a Program costs to render or run. Upper bounds
/// are stated where an exact figure isn't cheap; the runtime preallocates
/// from these (ADR 0005).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResourceEstimates {
    /// Total render length in frames (mono frame count).
    pub frames: u64,
    /// Total note events across all tracks.
    pub events: u64,
    /// An upper bound on simultaneously sounding notes across the mix: the
    /// per-track maxima summed (tracks start together, so their peaks can
    /// coincide). Voice pools sized to this never steal.
    pub peak_voices: u32,
    /// The dominant render allocation: the stereo f32 output buffers
    /// (`frames × 2 channels × 4 bytes`). Everything else (per-track bounces)
    /// is bounded by the same order.
    pub memory_bytes: u64,
}

/// Why a serialized [`Program`] failed to load.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProgramError {
    /// The JSON didn't parse or didn't match the bundle shape.
    Json(String),
    /// The resolved document fails current-format validation.
    InvalidDocument(String),
    /// The bundle's format revision is unsupported.
    UnsupportedVersion {
        /// The bundle's revision.
        found: u32,
        /// This binary's [`PROGRAM_VERSION`].
        supported: u32,
    },
    /// The stored hash doesn't match the recomputed one — the bundle was
    /// hand-edited or corrupted (T3002).
    HashMismatch {
        /// The hash stored in the bundle.
        stored: u64,
        /// The hash recomputed from the versioned bundle content.
        computed: u64,
    },
}

impl std::fmt::Display for ProgramError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ProgramError::Json(e) => write!(f, "program JSON: {e}"),
            ProgramError::InvalidDocument(e) => write!(f, "program document: {e}"),
            ProgramError::UnsupportedVersion { found, supported } => write!(
                f,
                "T3001: unsupported program version {found}; expected {supported}"
            ),
            ProgramError::HashMismatch { stored, computed } => write!(
                f,
                "T3002: program hash mismatch (stored {stored:#018x}, computed {computed:#018x}) — \
                 the bundle was edited or corrupted; recompile the song"
            ),
        }
    }
}

impl std::error::Error for ProgramError {}

/// FNV-1a over bytes — the same primitive the golden corpus uses over sample
/// bits, here over canonical JSON.
fn fnv1a(bytes: &[u8]) -> u64 {
    let mut h: u64 = 0xCBF2_9CE4_8422_2325;
    for b in bytes {
        h ^= *b as u64;
        h = h.wrapping_mul(0x0000_0100_0000_01B3);
    }
    h
}

/// The canonical form of a resolved document (ADR 0003): UTF-8 JSON, object
/// keys sorted (the serde_json default map), no insignificant whitespace,
/// floats in shortest-round-trip form. Two equivalent songs — authored in
/// Rust or Python — serialize to the same bytes.
fn canonical_json<T: Serialize>(value: &T) -> Vec<u8> {
    // serde_json's Map is a BTreeMap without the preserve_order feature, so
    // to_value→to_string is already the canonicalization (sorted keys,
    // compact separators, ryu float formatting).
    let value = serde_json::to_value(value).expect("canonical content serializes");
    serde_json::to_string(&value)
        .expect("a resolved document serializes")
        .into_bytes()
}

/// The canonical content hash of a resolved document: FNV-1a over its
/// canonical JSON. Independent of the authoring structure and of serialization
/// formatting — equivalent songs hash equal, from Rust or Python alike.
pub fn content_hash(doc: &SoundDoc) -> u64 {
    fnv1a(&canonical_json(doc))
}

impl Program {
    /// Recompute the integrity hash over all serialized semantic fields except `hash`.
    pub(crate) fn computed_hash(&self) -> u64 {
        let mut value = serde_json::to_value(self).expect("a program serializes");
        value
            .as_object_mut()
            .expect("a program serializes as an object")
            .remove("hash");
        fnv1a(&canonical_json(&value))
    }

    /// Render the full program to mono samples through the standard engine.
    pub fn render_mono(&self) -> Vec<f32> {
        render::render(&self.doc)
    }

    /// Render the full program to a stereo pair. A compiled song always has a
    /// stereo mix; a defensively duplicated mono pair is returned if it
    /// somehow doesn't.
    pub fn render_stereo(&self) -> (Vec<f32>, Vec<f32>) {
        render::render_product(&self.doc).into_stereo()
    }

    /// Render a frame range `[start, end)` as a stereo pair — a slice of the
    /// full render, so a note or tail crossing the range boundary sounds
    /// exactly as it does in the full mix (this is why ranges render through,
    /// not from, the boundary). Out-of-range requests clamp.
    pub fn render_range_frames(&self, start: u64, end: u64) -> (Vec<f32>, Vec<f32>) {
        let (l, r) = self.render_stereo();
        let start = (start as usize).min(l.len());
        let end = (end as usize).min(l.len()).max(start);
        (l[start..end].to_vec(), r[start..end].to_vec())
    }

    /// Render a bar range `[start_bar, end_bar)` through the program's meter
    /// map (see [`Self::render_range_frames`]).
    pub fn render_range_bars(&self, start_bar: u32, end_bar: u32) -> (Vec<f32>, Vec<f32>) {
        let transport = crate::runtime::Transport::for_program(self);
        self.render_range_frames(
            transport.frame_at_bar(start_bar),
            transport.frame_at_bar(end_bar),
        )
    }

    /// Render per-track and per-bus stereo stems (pre-master-chain — see
    /// [`render::Stem`]): every track stem plus every bus stem, in
    /// declaration order. Muted tracks are silent stems.
    pub fn render_stems(&self) -> Vec<render::Stem> {
        render::render_stems(&self.doc).unwrap_or_default()
    }

    /// Streaming blockers, derived from the resolved document.
    pub fn warnings(&self) -> Vec<Diagnostic> {
        blocker_warnings(&self.doc)
    }

    /// Whether the resolved document streams natively (no warnings is the
    /// same signal — blockers are the only warnings compilation produces).
    pub fn is_streamable(&self) -> bool {
        StreamGraph::blockers(&self.doc).is_empty()
    }

    /// The machine-readable capability list: what this program can do on a
    /// host — `"offline-render"` and `"stems"` always; `"streaming"` when the
    /// resolved document streams natively. Derived from the warnings (a pure
    /// function of the document), so it's identical from any loader.
    pub fn capabilities(&self) -> Vec<&'static str> {
        let mut caps = vec!["offline-render", "stems"];
        if self.is_streamable() {
            caps.push("streaming");
        }
        caps
    }

    /// Serialize the bundle (compact JSON; stable field order from the struct).
    pub fn to_json(&self) -> String {
        serde_json::to_string(self).expect("a program serializes")
    }

    /// Load a current bundle and verify its integrity and resolved document.
    pub fn from_json(json: &str) -> Result<Program, ProgramError> {
        let program: Program =
            serde_json::from_str(json).map_err(|e| ProgramError::Json(e.to_string()))?;
        program.verify()?;
        Ok(program)
    }

    pub(crate) fn verify(&self) -> Result<(), ProgramError> {
        if self.program_version != PROGRAM_VERSION {
            return Err(ProgramError::UnsupportedVersion {
                found: self.program_version,
                supported: PROGRAM_VERSION,
            });
        }
        let computed = self.computed_hash();
        if computed != self.hash {
            return Err(ProgramError::HashMismatch {
                stored: self.hash,
                computed,
            });
        }
        self.doc
            .validate()
            .map_err(|e| ProgramError::InvalidDocument(e.to_string()))?;
        if self.target == crate::song::CompileTarget::Runtime && !self.is_streamable() {
            return Err(ProgramError::InvalidDocument(
                "runtime target requires native streaming".into(),
            ));
        }
        Ok(())
    }
}

/// The streaming blockers of a resolved document, as warnings with per-kind
/// codes in the T15xx band.
pub(crate) fn blocker_warnings(doc: &SoundDoc) -> Vec<Diagnostic> {
    /// The per-kind code; a mixer part reports its cause's code (the message
    /// already carries the track/bus context).
    fn code(b: &crate::streaming::StreamBlocker) -> &'static str {
        use crate::streaming::StreamBlocker as B;
        match b {
            B::Normalize => "T1501",
            B::LoopPlayback => "T1502",
            B::StereoTreatment => "T1503",
            B::NestedMixer => "T1504",

            B::Sampler => "T1506",
            B::ModulatedFilter => "T1507",
            B::OfflineEffect { .. } => "T1508",
            B::TracksPart { cause, .. } => code(cause),
        }
    }
    StreamGraph::blockers(doc)
        .into_iter()
        .map(|b| {
            Diagnostic::warning(code(&b), "doc", b.to_string()).with_remediation(
                "the offline render is unaffected; live playback uses the buffer-backed Player",
            )
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::song::{CompileOptions, Song, note};

    fn two_track_program() -> Program {
        let mut song = Song::new("prog", 120.0);
        song.add_track(
            "bass",
            crate::dsl::SeqWave::Bass,
            crate::dsl::Adsr {
                a: 0.005,
                d: 0.1,
                s: 0.8,
                r: 0.2,
                punch: 0.0,
            },
        );
        song.add_pattern("riff", 1, vec![note(0, 4, "C2"), note(8, 4, "G2")]);
        song.arrange("bass", "riff", 0);
        song.compile(&CompileOptions::default()).expect("compiles")
    }

    #[test]
    fn hash_is_canonical_regardless_of_field_order() {
        let program = two_track_program();
        let json = serde_json::to_string(&program.doc).unwrap();
        // Shuffle the top-level key order: parse to a Value, re-emit in a
        // different order, reparse — the hash must not move.
        let mut value: serde_json::Value = serde_json::from_str(&json).unwrap();
        let obj = value.as_object_mut().unwrap();
        let mut entries: Vec<_> = obj.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
        entries.reverse();
        *obj = entries.into_iter().collect();
        let reparsed: SoundDoc = serde_json::from_value(value).unwrap();
        assert_eq!(content_hash(&reparsed), content_hash(&program.doc));
        assert_eq!(program.computed_hash(), program.hash);
    }

    #[test]
    fn program_round_trips_through_json() {
        let program = two_track_program();
        let loaded = Program::from_json(&program.to_json()).expect("loads");
        assert_eq!(loaded.hash, program.hash);
        assert_eq!(loaded.warnings().len(), program.warnings().len());
        assert_eq!(loaded.target, program.target);
        assert_eq!(loaded.render_mono(), program.render_mono());
        // The capability list is machine-readable and derived on load.
        assert!(loaded.capabilities().contains(&"streaming"));
    }

    #[test]
    fn render_range_is_a_slice_of_the_full_render() {
        let program = two_track_program();
        let (l, r) = program.render_stereo();
        let (sl, sr) = program.render_range_frames(1000, 5000);
        assert_eq!(sl, l[1000..5000].to_vec());
        assert_eq!(sr, r[1000..5000].to_vec());
        // A bar range converts through the meter map.
        let (bl, _) = program.render_range_bars(0, 1);
        let transport = crate::runtime::Transport::for_program(&program);
        assert_eq!(bl.len(), transport.frame_at_bar(1) as usize);
        // Out-of-range clamps instead of panicking.
        let (cl, _) = program.render_range_frames(u64::MAX - 1, u64::MAX);
        assert!(cl.is_empty());
    }

    #[test]
    fn from_json_rejects_an_unsupported_revision() {
        let program = two_track_program();
        let mut value: serde_json::Value = serde_json::from_str(&program.to_json()).unwrap();
        for found in [0, PROGRAM_VERSION - 1, PROGRAM_VERSION + 1] {
            value["program_version"] = serde_json::json!(found);
            let err = Program::from_json(&serde_json::to_string(&value).unwrap()).unwrap_err();
            assert_eq!(
                err,
                ProgramError::UnsupportedVersion {
                    found,
                    supported: PROGRAM_VERSION
                }
            );
        }
    }

    #[test]
    fn from_json_catches_a_hand_edited_bundle() {
        let program = two_track_program();
        let mut value: serde_json::Value = serde_json::from_str(&program.to_json()).unwrap();
        value["doc"]["duration"] = serde_json::json!(9.0);
        let err = Program::from_json(&serde_json::to_string(&value).unwrap()).unwrap_err();
        assert!(matches!(err, ProgramError::HashMismatch { .. }));

        let mut value: serde_json::Value = serde_json::from_str(&program.to_json()).unwrap();
        value["meta"]["tempo_bpm"] = serde_json::json!(90.0);
        let err = Program::from_json(&serde_json::to_string(&value).unwrap()).unwrap_err();
        assert!(
            matches!(err, ProgramError::HashMismatch { .. }),
            "transport metadata is part of the bundle's integrity boundary"
        );
    }

    #[test]
    fn current_bundles_reject_invalid_documents_even_with_a_matching_hash() {
        for pin in ["engine", "version"] {
            let mut program = two_track_program();
            if pin == "engine" {
                program.doc.engine = 0;
            } else {
                program.doc.version = 1;
            }
            program.hash = program.computed_hash();
            assert!(matches!(
                Program::from_json(&program.to_json()),
                Err(ProgramError::InvalidDocument(_))
            ));
        }
    }

    #[test]
    fn estimates_bound_the_render() {
        let program = two_track_program();
        assert_eq!(program.estimates.events, 2);
        assert_eq!(program.estimates.peak_voices, 1);
        assert_eq!(
            program.estimates.frames,
            (program.doc.duration * program.doc.sample_rate as f32).round() as u64
        );
        assert_eq!(program.estimates.memory_bytes, program.estimates.frames * 8);
    }
}
