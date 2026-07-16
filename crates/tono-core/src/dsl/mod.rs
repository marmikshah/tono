//! The Tono synthesis-graph DSL.
//!
//! A [`SoundDoc`] is the canonical, declarative source of a sound. The AI agent
//! authors one of these; the renderer turns it into samples. Everything here is
//! `serde`-deserializable (the on-disk / wire format is JSON) and `JsonSchema`-
//! describable so a tool can self-correct against the schema.

mod node;
#[cfg(test)]
mod tests;
mod tracks;
mod validate;

pub use node::{BassKnobs, FmKnobs, Node, PianoKnobs, PluckKnobs, Sf2Knobs};
pub use tracks::{AutoLane, AutoPoint, AutoTarget, Track};
pub use validate::ValidateError;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Current DSL schema version. Stored on every doc so old graphs stay loadable
/// as the vocabulary evolves. Version 2 gives every mixer track its own
/// deterministic RNG stream (v1 threads one stream through the track list in
/// order, so editing one track shifts the noise content of its siblings).
pub const SCHEMA_VERSION: u32 = 2;

/// Current DSP-kernel (engine) revision. Distinct from [`SCHEMA_VERSION`]:
/// that versions the *document schema* (what fields exist and how the graph is
/// structured); this versions the *audio kernels* (how a node turns into
/// samples). Splitting them lets a quality-improving kernel change ship
/// WITHOUT altering the bytes of any document authored before it. A document's
/// `engine` is `0` when omitted — the original kernels every shipped sound was
/// rendered under, byte-identical forever. New documents are stamped with this
/// value, opting them into the current kernels (e.g. anti-aliased `drive`).
/// Revision 1 adds antiderivative anti-aliasing to [`Node::Drive`]. Revision 2
/// gives each `noise`/`dust` node its own structurally-seeded RNG (derived from
/// its position in the graph) instead of drawing from one shared, traversal-order
/// stream — decorrelating sibling noise and, crucially, letting the real-time
/// streaming renderer produce byte-identical randomness block-by-block.
/// Revision 3 upgrades the `piano` seq voice to an inharmonic additive model
/// (stretched partials, per-partial decay, a hammer-strike spectrum, and a
/// detuned unison pair) — a far richer grand than the two-operator FM of
/// engine ≤ 2, which stays bit-exact for older documents.
/// Revision 4 corrects the mixer output stage — loudness normalization
/// measures the stereo program jointly (one shared gain, preserving the
/// authored balance), uses sample-rate-correct gated BS.1770 loudness, and
/// limits against a real oversampled true-peak estimate — and seeds humanize
/// jitter per note, so chords stop sharing one timing/velocity offset.
pub const ENGINE_VERSION: u32 = 4;

// Serde `default = "..."` requires free functions. Values with non-obvious
// origins: haas 12 ms sits in the precedence-effect sweet spot, ceiling
// −1 dBTP is the common streaming-safe true-peak ceiling.
fn default_sample_rate() -> u32 {
    44_100
}
fn default_duration() -> f32 {
    0.3
}
fn default_gain() -> f32 {
    1.0
}
fn default_haas_ms() -> f32 {
    12.0
}
fn default_wide_amount() -> f32 {
    0.6
}
fn default_ceiling_dbtp() -> f32 {
    -1.0
}
fn default_crossfade() -> f32 {
    0.1
}
fn default_mode_decay() -> f32 {
    0.4
}

/// A complete sound: metadata plus a single root node.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SoundDoc {
    /// Human-readable label for the sound (e.g. `"laser_zap"`).
    pub name: String,
    /// Length of the rendered sound in seconds.
    #[serde(default = "default_duration")]
    pub duration: f32,
    /// Output sample rate in Hz.
    #[serde(default = "default_sample_rate")]
    pub sample_rate: u32,
    /// Seed for any stochastic node (noise). Same seed ⇒ identical audio.
    #[serde(default)]
    pub seed: u64,
    /// DSL schema version. Omitted ⇒ 1, the semantics documents were authored
    /// under before versioning mattered; the authoring tools stamp new
    /// documents with the current [`SCHEMA_VERSION`]. Documents from a newer
    /// tono are rejected by `validate` instead of silently misrendered.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<u32>,
    /// DSP-kernel revision (see [`ENGINE_VERSION`]). Omitted ⇒ 0, the original
    /// kernels — so every existing document renders byte-for-byte as before.
    /// The authoring tools stamp new documents with the current
    /// [`ENGINE_VERSION`]; raising a document's `engine` opts it into newer,
    /// higher-quality kernels (anti-aliased `drive`, …) and DOES change its
    /// output. A document from a newer tono (engine > `ENGINE_VERSION`) is
    /// rejected by `validate` rather than silently misrendered.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub engine: Option<u32>,
    /// Optional stereo treatment applied to the final mono render. Defaults to
    /// mono (game SFX are usually authored mono and spatialised by the engine;
    /// use stereo for BGM, ambience, and UI stingers).
    #[serde(default)]
    pub stereo: Stereo,
    /// Optional output-stage loudness normalization + true-peak limiting. When
    /// set with `target_lufs`, the final render is gain-matched to that
    /// integrated loudness, then brick-wall limited so the inter-sample (true)
    /// peak never exceeds `ceiling_dbtp`. Leave unset for the default behaviour
    /// (a transparent −0.1 dBFS sample-peak safety limit only). Use it to ship a
    /// level-matched set: pick one target (e.g. −16 LUFS for SFX) for the pack.
    #[serde(default)]
    pub normalize: Option<Normalize>,
    /// Playback intent. `oneshot` (default) renders the sound as-is. `loop`
    /// extracts the loop region and equal-power crossfades its tail into its
    /// head so the rendered file repeats seamlessly — the right mode for
    /// ambience beds, engine drones, and BGM. The exported WAV carries a `smpl`
    /// loop chunk so engines (Godot / Unity / FMOD) loop at the sample-accurate
    /// points without manual setup.
    #[serde(default)]
    pub playback: Playback,
    /// The signal graph. Usually a `mix`, `mul`, or `chain`.
    pub root: Node,
}

impl SoundDoc {
    /// A new document around `root`, stamped with the current
    /// [`SCHEMA_VERSION`] and [`ENGINE_VERSION`] (this is the authoring
    /// constructor — new sounds get the current kernels) and every other
    /// field at its serde default: 0.3 s, 44 100 Hz, seed 0, mono, one-shot.
    pub fn new(name: impl Into<String>, root: Node) -> Self {
        SoundDoc {
            name: name.into(),
            duration: default_duration(),
            sample_rate: default_sample_rate(),
            seed: 0,
            version: Some(SCHEMA_VERSION),
            engine: Some(ENGINE_VERSION),
            stereo: Stereo::default(),
            normalize: None,
            playback: Playback::default(),
            root,
        }
    }
}

/// How the rendered sound is meant to be played back.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "mode", rename_all = "lowercase")]
pub enum Playback {
    /// Play once (default).
    #[default]
    OneShot,
    /// Seamless loop. The renderer extracts the region `[start_secs, end_secs)`
    /// and crossfades its last `crossfade_secs` (equal-power) onto its head, so
    /// the rendered buffer repeats with no click. The output is the loop body
    /// (shorter than the source by the crossfade), and the WAV gets a `smpl`
    /// loop spanning the whole file.
    #[serde(rename = "loop")]
    Loop {
        /// Loop start in seconds (default 0).
        #[serde(default)]
        start_secs: f32,
        /// Loop end in seconds (default: end of the rendered buffer).
        #[serde(default)]
        end_secs: Option<f32>,
        /// Equal-power crossfade length in seconds (default 0.1). Longer hides
        /// bigger discontinuities but shortens the loop more.
        #[serde(default = "default_crossfade")]
        crossfade_secs: f32,
    },
}

/// Output-stage loudness normalization + true-peak limiting.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema)]
pub struct Normalize {
    /// Target integrated loudness in LUFS (e.g. −16 for SFX, −14 for music).
    /// The render is gain-matched to hit this before limiting. Omit to skip
    /// loudness matching and only apply the true-peak ceiling.
    #[serde(default)]
    pub target_lufs: Option<f32>,
    /// True-peak ceiling in dBTP. The output is limited so its inter-sample peak
    /// stays at or below this. Defaults to −1.0.
    #[serde(default = "default_ceiling_dbtp")]
    pub ceiling_dbtp: f32,
}

/// Stereo treatment for the final render.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "mode", rename_all = "lowercase")]
pub enum Stereo {
    /// Mono — both channels identical (default).
    #[default]
    Mono,
    /// Haas precedence widening: one channel delayed by `ms` (1..40), shifting
    /// the apparent position and adding width. `pan` (-1 left .. 1 right) sets
    /// which side leads.
    Haas {
        /// Inter-channel delay in milliseconds.
        #[serde(default = "default_haas_ms")]
        ms: f32,
        /// Lead side, −1 (left) .. 1 (right).
        #[serde(default)]
        pan: f32,
    },
    /// Pseudo-stereo: decorrelate the channels for width on pads / BGM.
    Wide {
        /// Width amount, 0 (mono) .. 1 (fully decorrelated).
        #[serde(default = "default_wide_amount")]
        amount: f32,
    },
}

/// A numeric parameter that is either a constant or a time-varying modulator.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(untagged)]
pub enum Value {
    /// A constant value (e.g. a fixed frequency in Hz).
    Const(f32),
    /// A musical pitch as a string: a note name like `"A4"`, `"C#3"`, `"Gb5"`,
    /// `"F#-1"`, or a MIDI number like `"midi:69"` / `"m69"`. Resolves to Hz
    /// (A4 = 440, 12-TET) — so melodies read musically instead of as raw Hz.
    Note(String),
    /// A modulator that produces a value per sample.
    Modulated(Modulator),
}

impl From<f32> for Value {
    /// A constant — `"freq": 440.0.into()`.
    fn from(v: f32) -> Self {
        Value::Const(v)
    }
}

impl From<&str> for Value {
    /// A note name (`"C4"`, `"F#3"`, `"midi:69"`) — resolved to Hz at render.
    fn from(name: &str) -> Self {
        Value::Note(name.to_string())
    }
}

impl From<String> for Value {
    /// A note name (see [`note_to_hz`]).
    fn from(name: String) -> Self {
        Value::Note(name)
    }
}

impl From<Modulator> for Value {
    fn from(m: Modulator) -> Self {
        Value::Modulated(m)
    }
}

/// Parse a musical pitch into Hz: a note name (`"A4"`, `"C#3"`, `"Gb5"`,
/// `"F#-1"`; octave defaults to 4) or a MIDI number (`"midi:69"` / `"m69"`).
/// A4 = 440 Hz, 12-tone equal temperament. Returns `None` if unparseable.
pub fn note_to_hz(s: &str) -> Option<f32> {
    let s = s.trim();
    if s.is_empty() {
        return None;
    }
    // MIDI forms: "midi:69" or "m69".
    if let Some(num) = s
        .strip_prefix("midi:")
        .or_else(|| s.strip_prefix(['m', 'M']))
        && let Ok(n) = num.trim().parse::<f32>()
    {
        return Some(midi_to_hz(n));
    }
    // Note name: letter, optional #/b accidentals, optional octave (default 4).
    let mut chars = s.chars().peekable();
    let mut semis: i32 = match chars.next()?.to_ascii_uppercase() {
        'C' => 0,
        'D' => 2,
        'E' => 4,
        'F' => 5,
        'G' => 7,
        'A' => 9,
        'B' => 11,
        _ => return None,
    };
    loop {
        match chars.peek() {
            Some('#') => semis += 1,
            Some('b') => semis -= 1,
            _ => break,
        }
        chars.next();
    }
    let rest: String = chars.collect();
    let octave: i32 = if rest.is_empty() {
        4
    } else {
        rest.parse().ok()?
    };
    Some(midi_to_hz(((octave + 1) * 12 + semis) as f32))
}

fn midi_to_hz(m: f32) -> f32 {
    440.0 * 2f32.powf((m - 69.0) / 12.0)
}

/// Interpolation curve for a [`Modulator::Slide`].
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum Curve {
    /// Linear interpolation.
    #[default]
    Lin,
    /// Exponential interpolation (perceptually natural for pitch/cutoff sweeps).
    Exp,
}

/// Oscillator shape for an LFO modulator.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum Shape {
    /// Sine wave.
    #[default]
    Sine,
    /// Square wave.
    Square,
    /// Triangle wave.
    Triangle,
    /// Sawtooth wave.
    Saw,
}

/// A time-varying parameter value. Externally tagged: `{ "slide": {...} }`.
#[non_exhaustive]
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub enum Modulator {
    /// Glide from `from` to `to` over `secs`, then hold at `to`.
    #[serde(rename = "slide")]
    Slide {
        /// Start value.
        from: f32,
        /// End value.
        to: f32,
        /// Glide time in seconds.
        secs: f32,
        /// Interpolation curve.
        #[serde(default)]
        curve: Curve,
    },
    /// Low-frequency oscillation around `center` (vibrato / tremolo).
    #[serde(rename = "lfo")]
    Lfo {
        /// Oscillator shape.
        #[serde(default)]
        shape: Shape,
        /// Oscillation rate in Hz.
        rate: f32,
        /// Peak deviation from `center`.
        depth: f32,
        /// Mean value the LFO oscillates around.
        center: f32,
    },
    /// Step through `steps` at `rate` steps/sec, looping (arpeggio / blip table).
    #[serde(rename = "arp")]
    Arp {
        /// Sequence of values to cycle through.
        steps: Vec<f32>,
        /// Steps per second.
        rate: f32,
    },
    /// An ADSR envelope mapped onto a parameter range: the value rides from
    /// `from` (envelope = 0) to `to` (envelope = 1). This is the modulation
    /// behind filter envelopes (cutoff `from` high `to` low), pitch envelopes,
    /// and amplitude shaping of any param. The shape is time-based, not slide.
    #[serde(rename = "env")]
    EnvMod {
        /// Envelope shape.
        #[serde(flatten)]
        adsr: Adsr,
        /// Parameter value when the envelope is at 0.
        from: f32,
        /// Parameter value when the envelope is at 1.
        to: f32,
    },
    /// Smooth random walk between `from` and `to`, drifting at `rate` new
    /// targets per second (smoothstep-interpolated). The organic, NON-periodic
    /// motion the other modulators lack — wind gusting on a filter cutoff,
    /// fire flicker on a gain, drifting detune. Deterministic and edit-stable:
    /// the walk is seeded only from this modulator's own fields, so it never
    /// shifts when sibling nodes change. Give two `rand`s different `seed`s (or
    /// rates) to decorrelate them.
    #[serde(rename = "rand")]
    Rand {
        /// Lower bound of the walk.
        from: f32,
        /// Upper bound of the walk.
        to: f32,
        /// New random targets per second (low = slow drift, high = jittery).
        rate: f32,
        /// Decorrelation seed; defaults to 0. Distinct values give independent
        /// walks for the same `from`/`to`/`rate`.
        #[serde(default)]
        seed: u64,
    },
}

/// Waveshaper curve for [`Node::Drive`].
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum DriveShape {
    /// Smooth `tanh` saturation (warm).
    #[default]
    Tanh,
    /// Hard clipping (aggressive, square-ish).
    Hard,
    /// Wavefolding (bright, metallic harmonics).
    Fold,
}

/// Spectral colour of a [`Node::Noise`] source.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum NoiseColor {
    /// Flat spectrum (bright, hissy).
    #[default]
    White,
    /// −3 dB/octave (warm; wind, rumble, surf).
    Pink,
    /// −6 dB/octave (dark; distant booms, low rumble).
    Brown,
}

/// Oscillator shape for a [`Node::Super`] unison oscillator.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum SuperWave {
    /// Sawtooth (the classic supersaw).
    #[default]
    Sawtooth,
    /// Square / pulse.
    Square,
}

/// Oscillator choice for a [`Node::Seq`] note.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum SeqWave {
    /// Square / pulse (uses the seq's `duty`).
    #[default]
    Square,
    /// Triangle wave.
    Triangle,
    /// Sawtooth wave.
    Sawtooth,
    /// Sine wave.
    Sine,
    /// White noise (for drums / percussion).
    Noise,
    /// Two-operator FM struck per note (uses the seq's `fm_ratio` /
    /// `fm_index` / `fm_strike`): the modulation index starts bright at the
    /// attack and decays, like a hammer strike — e-piano, piano, bells,
    /// mallets. Louder notes (higher `gain`) ring brighter.
    Fm,
    /// Karplus-Strong plucked string (uses the seq's `pluck_decay`): a noise
    /// burst rings through a tuned feedback loop — guitar, harp, koto. Pitch
    /// is fixed per note (slides are ignored).
    Pluck,
    /// Acoustic piano model: two detuned FM strings per note with a hammer
    /// thump, velocity-sensitive brightness, and a natural pitch-dependent
    /// decay (bass strings ring for seconds, treble dies fast) — no
    /// parameters to set, play it like a piano. Set the seq env to
    /// `{a:0.002, s:1, r:0.2}` and let the instrument shape each note;
    /// `len` works like holding the key (with the pedal, longer).
    Piano,
    /// Electric piano (Rhodes-style): a soft FM body plus a bright metal
    /// tine that pings on the attack and fades fast. Velocity opens the
    /// tine — dig in for bark, play soft for bell-like warmth.
    Epiano,
    /// Tonewheel organ: drawbar harmonics (16′ 8′ 4′ 2⅔′ 2′) with a touch of
    /// percussion on the attack. Sustains at full level while the key is
    /// held — pair with env `{s:1}` and let `len` do the phrasing.
    Organ,
    /// String ensemble: three detuned band-limited saws per note with a slow
    /// bow swell and a mellowing lowpass — pads, sustained chords, swells.
    /// Notes bloom ~150 ms after the attack; write them slightly early.
    Strings,
    /// Fingered bass: a filtered saw whose cutoff snaps open with velocity
    /// and settles, over a solid sine sub. Punchy, dark, sits under a mix.
    Bass,
    /// Drum kit on the General MIDI map — the note's pitch picks the drum,
    /// not a frequency: `"midi:36"` kick, `38` snare, `42` closed hat,
    /// `46` open hat, `41..50` toms, `49` crash, `51` ride, `39` clap,
    /// `56` cowbell. Velocity (`gain`) sets the hit level.
    Kit,
    /// Pitched cowbell: two clashing saturated partials with a fast knock
    /// decay — played melodically it is THE phonk / Memphis lead. More
    /// cowbell.
    Cowbell,
    /// SoundFont sampler: plays the notes through real recorded instruments
    /// from an `.sf2` file (set the seq's `sf2` path and `sf2_preset` — the
    /// General MIDI program number, e.g. 0 grand piano, 32 acoustic bass,
    /// 48 strings; `sf2_bank: 128` selects the percussion bank, where notes
    /// follow the GM drum map). The biggest realism jump available: this is
    /// how DAWs sound real.
    Sampler,
}

/// Which drum-kit voicing the `kit` seq wave synthesizes. Every style follows
/// the same General MIDI note map; they differ only in how each drum is
/// synthesized. `Classic` is the original kit — omitting `kit` (or setting it to
/// `classic`) renders byte-identically to before this field existed.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum KitStyle {
    /// The original synthesized GM kit.
    #[default]
    Classic,
    /// A deeper, more realistic acoustic kit — punchier kick, tuned snare body,
    /// ringier toms, shimmery cymbals.
    Acoustic,
    /// Clean synthesized electronic drums — tight, punchy, crisp.
    Electronic,
    /// Roland TR-808 style — a long booming sub kick, ringy cowbell, snappy
    /// snare, tick-y percussion.
    #[serde(rename = "808")]
    Eight08,
}

/// An ADSR amplitude envelope. One shape, used in three places: the [`Node::Env`]
/// amplitude envelope, the per-note envelope of a [`Node::Seq`], and (with a
/// `from`/`to` range) the [`Modulator::EnvMod`] parameter envelope.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Adsr {
    /// Attack time in seconds.
    #[serde(default)]
    pub a: f32,
    /// Decay time in seconds.
    #[serde(default)]
    pub d: f32,
    /// Sustain level, 0..1.
    #[serde(default)]
    pub s: f32,
    /// Release time in seconds.
    #[serde(default)]
    pub r: f32,
    /// Initial transient boost, 0..1.
    #[serde(default)]
    pub punch: f32,
}

impl Adsr {
    /// An envelope with the four classic stages (`punch` 0). Attack/decay/
    /// release in seconds, sustain 0..1.
    pub fn new(a: f32, d: f32, s: f32, r: f32) -> Self {
        Adsr {
            a,
            d,
            s,
            r,
            punch: 0.0,
        }
    }
}

/// One resonant mode of a [`Node::Modal`] bank: a single damped sinusoidal
/// partial. A struck object's timbre is the set of these — their frequency
/// ratios say "metal" vs "wood" vs "glass", their decays say how it rings.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct Mode {
    /// Modal frequency in Hz.
    pub freq: f32,
    /// −60 dB ring time in seconds: how long this partial sustains after the
    /// strike. Higher modes usually decay faster than the fundamental.
    #[serde(default = "default_mode_decay")]
    pub decay: f32,
    /// Relative amplitude of this partial, 0..1.
    #[serde(default = "default_gain")]
    pub gain: f32,
}

/// One note in a [`Node::Seq`].
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SeqNote {
    /// Grid step at which the note starts (0-based).
    pub step: u32,
    /// Note length in grid steps.
    pub len: u32,
    /// Pitch in Hz (a constant, or a modulator such as a `slide` for a glide /
    /// pitched-drum thump). Ignored when the seq wave is `noise`.
    pub pitch: Value,
    /// Note velocity / level, 0..1.
    #[serde(default = "default_gain")]
    pub gain: f32,
}

impl SoundDoc {
    /// The schema version this document's render semantics follow (omitted ⇒ 1).
    pub fn effective_version(&self) -> u32 {
        self.version.unwrap_or(1)
    }

    /// The DSP-kernel revision this document renders under (omitted ⇒ 0, the
    /// original kernels). Gates byte-changing kernel upgrades so old documents
    /// stay bit-exact; see [`ENGINE_VERSION`].
    pub fn effective_engine(&self) -> u32 {
        self.engine.unwrap_or(0)
    }

    /// Every SoundFont path the document references (each `seq` with
    /// `wave: "sampler"` and a non-empty `sf2`). [`validate`](Self::validate)
    /// is filesystem-free — the core is pure compute — so a *loader* (the CLI,
    /// the Python bindings, a game's asset pipeline) calls this after
    /// validation to check the files exist and fail loud at load time.
    pub fn sf2_paths(&self) -> Vec<&str> {
        fn walk<'doc>(node: &'doc Node, out: &mut Vec<&'doc str>) {
            match node {
                Node::Seq { wave, sf2, .. } => {
                    if *wave == SeqWave::Sampler && !sf2.sf2.is_empty() {
                        out.push(sf2.sf2.as_str());
                    }
                }
                Node::Mix { inputs } | Node::Mul { inputs } => {
                    inputs.iter().for_each(|n| walk(n, out));
                }
                Node::Chain { stages } => stages.iter().for_each(|n| walk(n, out)),
                Node::Duck { trigger, .. } => walk(trigger, out),
                Node::Tracks { tracks, master } => {
                    tracks.iter().for_each(|t| walk(&t.node, out));
                    master.iter().for_each(|n| walk(n, out));
                }
                _ => {}
            }
        }
        let mut out = Vec::new();
        walk(&self.root, &mut out);
        out
    }

    /// Backfill missing track ids deterministically (`layer_<position>`,
    /// suffixed on collision with explicit ids). Runs at the build chokepoint
    /// so every persisted mixer document carries addressable layers; the rule
    /// is positional, so replaying a journal mints identical ids. Returns true
    /// if anything changed.
    pub fn ensure_track_ids(&mut self) -> bool {
        let Node::Tracks { tracks, .. } = &mut self.root else {
            return false;
        };
        let used: std::collections::HashSet<String> =
            tracks.iter().filter_map(|t| t.id.clone()).collect();
        let mut changed = false;
        for (i, t) in tracks.iter_mut().enumerate() {
            if t.id.is_none() {
                let mut id = format!("layer_{i}");
                let mut n = 2;
                while used.contains(&id) {
                    id = format!("layer_{i}_{n}");
                    n += 1;
                }
                t.id = Some(id);
                changed = true;
            }
        }
        changed
    }
}
