//! The instrument's data model: pitch mapping, play mode, modulation, and the
//! serializable [`InstrumentDesign`].

use serde::{Deserialize, Serialize};

use crate::dsl::{Adsr, Node};
use crate::patch::Patch;

use super::note::Note;

/// How a note sets an instrument's pitch.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum PitchMap {
    /// Set this named patch parameter to the note's frequency (Hz). Precise — the
    /// patch author decides exactly what the pitch drives.
    Param(String),
    /// Transpose every source frequency in the graph by `note.freq() /
    /// reference.freq()`. Turns *any* sound into a playable instrument with no
    /// pitch param required.
    Transpose {
        /// The note the patch is authored at (plays the graph unchanged).
        reference: Note,
    },
}

/// How notes share voices.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum PlayMode {
    /// Every note is its own voice — chords, the default.
    #[default]
    Poly,
    /// One voice at a time (leads, bass). A new note steals the voice and glides
    /// to it (see [`InstrumentDesign::glide_secs`]); releasing a note falls back
    /// to the most-recent one still held (last-note priority). `legato` keeps the
    /// amp envelope running when a note arrives while another is held — a smooth,
    /// connected line rather than a re-struck one.
    Mono {
        /// Keep the amp envelope running when a note arrives while another is
        /// held — a smooth, connected line rather than a re-struck one.
        legato: bool,
    },
}

/// Instrument-level modulation — LFOs that make a voice breathe. All off by
/// default (an all-zero `Modulation` leaves the render byte-identical). Driven
/// live at control rate, so it works on any instrument without re-authoring it.
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize)]
pub struct Modulation {
    /// Vibrato (pitch LFO) rate in Hz.
    #[serde(default)]
    pub vibrato_rate: f32,
    /// Vibrato depth in cents (0 = off).
    #[serde(default)]
    pub vibrato_cents: f32,
    /// Tremolo (amplitude LFO) rate in Hz.
    #[serde(default)]
    pub tremolo_rate: f32,
    /// Tremolo depth, 0..1 (0 = off).
    #[serde(default)]
    pub tremolo_depth: f32,
    /// Filter-wobble (cutoff LFO) rate in Hz.
    #[serde(default)]
    pub filter_rate: f32,
    /// Filter-wobble depth in octaves of cutoff sweep (0 = off).
    #[serde(default)]
    pub filter_octaves: f32,
}

impl Modulation {
    pub(super) fn is_active(&self) -> bool {
        self.vibrato_cents > 0.0 || self.tremolo_depth > 0.0 || self.filter_octaves > 0.0
    }
}

/// The recipe that makes a [`Patch`] playable. Serializable, so an instrument is
/// a saveable/recallable preset (patch + envelope + pitch map + master).
#[non_exhaustive]
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct InstrumentDesign {
    /// The sound: a graph + its named params (authored as a sustaining voice).
    pub patch: Patch,
    /// The gated amplitude envelope applied per note (note-on → note-off).
    pub amp: Adsr,
    /// How a note maps to pitch.
    pub pitch: PitchMap,
    /// An optional param driven by velocity (e.g. filter cutoff for brightness).
    pub velocity_param: Option<String>,
    /// Maximum simultaneous voices before the oldest is stolen.
    pub max_voices: usize,
    /// A shared master effect chain applied to the summed voices (one reverb/delay
    /// for the whole instrument, not one per voice — so a tail outlives its note).
    /// Must be streamable processor nodes.
    pub master: Vec<Node>,
    /// Poly (default) or mono/legato.
    #[serde(default)]
    pub mode: PlayMode,
    /// Portamento time in seconds: how long a note glides to the next in mono
    /// mode (0 = instant, no glide). Approximate — a one-pole ease, so the pitch
    /// arrives asymptotically.
    #[serde(default)]
    pub glide_secs: f32,
    /// Unison: detuned copies stacked per note for a fat, wide sound (1 = off).
    #[serde(default = "one")]
    pub unison: usize,
    /// Total detune spread across the unison stack, in cents (e.g. 20).
    #[serde(default)]
    pub detune_cents: f32,
    /// Stereo spread of the unison copies, 0 (mono) .. 1 (hard L↔R).
    #[serde(default)]
    pub unison_width: f32,
    /// LFO modulation (vibrato / tremolo / filter wobble). Off by default.
    #[serde(default)]
    pub modulation: Modulation,
}

fn one() -> usize {
    1
}

impl InstrumentDesign {
    /// A sensible default design for `patch`: a gentle amp envelope, 16 voices,
    /// and pitch by the param named `"pitch"` if the patch has one, else
    /// transpose from middle C.
    pub fn new(patch: Patch) -> Self {
        let has_pitch = patch.params.iter().any(|p| p.name == "pitch");
        InstrumentDesign {
            pitch: if has_pitch {
                PitchMap::Param("pitch".into())
            } else {
                PitchMap::Transpose {
                    reference: Note::C4,
                }
            },
            amp: Adsr {
                a: 0.005,
                d: 0.08,
                s: 0.7,
                r: 0.12,
                punch: 0.0,
            },
            velocity_param: None,
            max_voices: 16,
            master: Vec::new(),
            mode: PlayMode::Poly,
            glide_secs: 0.0,
            unison: 1,
            detune_cents: 0.0,
            unison_width: 0.0,
            modulation: Modulation::default(),
            patch,
        }
    }

    /// Set the shared master effect chain (builder style) — e.g. one reverb for
    /// the whole instrument instead of per voice.
    pub fn with_master(mut self, master: Vec<Node>) -> Self {
        self.master = master;
        self
    }

    /// Set the amplitude envelope (builder style).
    pub fn with_amp(mut self, amp: Adsr) -> Self {
        self.amp = amp;
        self
    }

    /// Set the pitch mapping (builder style).
    pub fn with_pitch(mut self, pitch: PitchMap) -> Self {
        self.pitch = pitch;
        self
    }

    /// Drive a named param from note velocity, mapped across the param's declared
    /// `[min, max]` (e.g. a filter cutoff for velocity → brightness).
    pub fn with_velocity_param(mut self, name: impl Into<String>) -> Self {
        self.velocity_param = Some(name.into());
        self
    }

    /// Set the maximum simultaneous voices (at least 1).
    pub fn with_max_voices(mut self, max: usize) -> Self {
        self.max_voices = max.max(1);
        self
    }

    /// Set the play mode — poly, or mono/legato (builder style).
    pub fn with_mode(mut self, mode: PlayMode) -> Self {
        self.mode = mode;
        self
    }

    /// Set the portamento glide time in seconds for mono mode (builder style).
    pub fn with_glide(mut self, secs: f32) -> Self {
        self.glide_secs = secs.max(0.0);
        self
    }

    /// Stack `count` detuned copies per note across `cents` of detune, spread
    /// `width` (0..1) across the stereo field — a fat, wide unison (builder
    /// style). `count == 1` is no unison.
    pub fn with_unison(mut self, count: usize, cents: f32, width: f32) -> Self {
        self.unison = count.max(1);
        self.detune_cents = cents.max(0.0);
        self.unison_width = width.clamp(0.0, 1.0);
        self
    }

    /// Add vibrato — a pitch LFO at `rate` Hz, `cents` deep (builder style).
    pub fn with_vibrato(mut self, rate: f32, cents: f32) -> Self {
        self.modulation.vibrato_rate = rate;
        self.modulation.vibrato_cents = cents.max(0.0);
        self
    }

    /// Add tremolo — an amplitude LFO at `rate` Hz, `depth` 0..1 (builder style).
    pub fn with_tremolo(mut self, rate: f32, depth: f32) -> Self {
        self.modulation.tremolo_rate = rate;
        self.modulation.tremolo_depth = depth.clamp(0.0, 1.0);
        self
    }

    /// Add filter wobble — a cutoff LFO at `rate` Hz sweeping `octaves` wide
    /// (builder style). Needs a filter in the patch to hear.
    pub fn with_wobble(mut self, rate: f32, octaves: f32) -> Self {
        self.modulation.filter_rate = rate;
        self.modulation.filter_octaves = octaves.max(0.0);
        self
    }
}
