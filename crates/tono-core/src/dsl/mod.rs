//! The Tono synthesis-graph DSL.
//!
//! A [`SoundDoc`] is the canonical, declarative source of a sound. An authoring
//! tool creates one; the renderer turns it into samples. Everything here is
//! `serde`-deserializable (the on-disk / wire format is JSON) and `JsonSchema`-
//! describable so a tool can self-correct against the schema.

mod node;
#[cfg(test)]
mod tests;
mod tracks;
mod validate;

pub use node::{BassKnobs, Children, ChildrenMut, FmKnobs, Node, PianoKnobs, PluckKnobs, Sf2Knobs};
pub use tracks::{AutoCurve, AutoLane, AutoPoint, AutoTarget, Bus, Send, Sidechain, Track};
pub use validate::ValidateError;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// The supported document schema revision.
pub const SCHEMA_VERSION: u32 = 2;

/// The supported deterministic DSP-kernel revision.
pub const ENGINE_VERSION: u32 = 5;

pub(crate) fn default_version() -> u32 {
    SCHEMA_VERSION
}
pub(crate) fn default_engine() -> u32 {
    ENGINE_VERSION
}

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
    /// Document schema revision. Omitted JSON pins use the current schema.
    #[serde(default = "default_version")]
    pub version: u32,
    /// DSP-kernel revision. Only the current revision is supported.
    #[serde(default = "default_engine")]
    pub engine: u32,
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
            version: SCHEMA_VERSION,
            engine: ENGINE_VERSION,
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
    /// Haas precedence widening: one channel delayed by `ms` ([0.5, 40], validated), shifting
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

/// Resolve a note name (`C4`, `F#3`, `midi:69`, `m69`) to Hz with deterministic
/// pitch conversion. Octave defaults to 4. Invalid or out-of-range names return None.
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
        return midi_to_hz(n);
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
    // i64 headroom: huge octaves ("A200000000") would overflow i32 arithmetic.
    midi_to_hz(((octave as i64 + 1) * 12 + semis as i64) as f32)
}

fn midi_to_hz(m: f32) -> Option<f32> {
    let hz = 440.0 * crate::dsp::powf(2.0, (m - 69.0) / 12.0);
    // Reject pitches that would poison the render: non-finite or non-positive
    // Hz turns oscillator phase accumulators to NaN, and anything far above
    // the highest supported Nyquist (96 kHz at 192 kHz sr) is an authoring
    // error, not a sound.
    (hz.is_finite() && hz > 0.0 && hz <= 100_000.0).then_some(hz)
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

/// Table set of a [`Node::Wavetable`] morphing oscillator: an ordered set of
/// single-cycle waves that `position` (0..1) crossfades across. Each sub-wave
/// is generated at node build time by additive synthesis, band-limited to 32
/// partials (darker sub-waves use fewer), sampled into a 2048-sample table —
/// zero assets, fully deterministic.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum WavetableKind {
    /// sine → triangle → square → saw: the classic dark-to-bright morph.
    #[default]
    Basic,
    /// A saw that grows its harmonic count (1 → 2 → 4 → 8 → 16 → 32 partials):
    /// a pure brightness ramp.
    Harmonics,
    /// Vowel-ish fixed formant stacks a → e → i → o → u (fundamental plus two
    /// partials tuned to formant centres, voiced against a ~110 Hz reference).
    /// Sweep `position` slowly for vocal morphs.
    Formant,
    /// Sparse, cluster-like partial stacks (missing fundamentals, wide gaps)
    /// that read metallic / clangorous while staying perfectly periodic.
    Metallic,
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
    /// Brass section: two detuned band-limited saws through a lowpass whose
    /// cutoff swells open over the first ~70 ms — the "blat" of a horn
    /// attack. Velocity (`gain`) opens the filter further: dig in for a
    /// bright stab, play soft for a mellow swell. Sustains while held —
    /// pair with env `{s:1}` and let `len` phrase.
    Brass,
    /// Concert flute: a sine with a vibrato (~5.5 Hz) that fades in over
    /// the first ~150 ms, over a breath of lowpassed air noise. Velocity
    /// (`gain`) adds breath and edge. Sustains while held — write long
    /// notes and let `len` shape the phrase.
    Flute,
    /// Marimba-like mallet: a warm sine fundamental with two wooden strike
    /// partials that die in tens of milliseconds — the "thok" of a mallet
    /// hit. Velocity (`gain`) brightens the strike. Woodier and
    /// shorter-lived than `epiano`; use a short env and space the notes.
    Mallet,
    /// Struck bell: inharmonic partials (1, 2.02, 2.74, 4.07, 5.43) with
    /// per-partial decays — the highs die first, the hum rings on — plus a
    /// slightly detuned twin of the fundamental whose slow beating is the
    /// shimmer. Velocity (`gain`) scales the hit. Long natural ring: give
    /// the notes room.
    Bell,
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
/// synthesized. Omitting `kit` selects `Classic`.
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

/// A tempo change at an exact beat position: from `at` until the
/// next change, the tempo is `bpm`. In a [`Node::Seq`]'s `tempo_map` the
/// first point must sit at beat 0 — an empty map is the constant-tempo
/// `bpm` behavior.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema, PartialEq)]
pub struct TempoPoint {
    /// The exact beat the change takes effect at (a normalized rational —
    /// tuplets and fractional bar lines stay exact).
    pub at: crate::units::Beat,
    /// The new tempo in beats per minute.
    pub bpm: f32,
}

/// Seconds elapsed at `beat` under a tempo map — the segment walk in f64,
/// the conversion shared by the renderer and compiler.
/// Degenerate tempos floor at 1 BPM, like the seq's own clamp.
/// The map must be non-empty and start at beat 0 (validation enforces both).
pub fn tempo_map_seconds_at(map: &[TempoPoint], beat: f64) -> f64 {
    let mut secs = 0.0;
    let mut prev_beat = 0.0f64;
    let mut bpm = (map[0].bpm as f64).max(1.0);
    for p in &map[1..] {
        let at = p.at.to_f64();
        if beat <= at {
            break;
        }
        secs += (at - prev_beat) * 60.0 / bpm;
        prev_beat = at;
        bpm = (p.bpm as f64).max(1.0);
    }
    secs + (beat - prev_beat) * 60.0 / bpm
}

/// The tempo in effect at `beat` under a tempo map (≥ 1 BPM, floored).
pub fn tempo_map_bpm_at(map: &[TempoPoint], beat: f64) -> f64 {
    let mut bpm = (map[0].bpm as f64).max(1.0);
    for p in &map[1..] {
        if beat < p.at.to_f64() {
            break;
        }
        bpm = (p.bpm as f64).max(1.0);
    }
    bpm
}

/// The inverse of [`tempo_map_seconds_at`]: the beat position at `seconds`
/// under the map, walking exact segment boundaries in f64.
/// The map must be non-empty and start at beat 0.
pub fn tempo_map_beat_at_seconds(map: &[TempoPoint], seconds: f64) -> f64 {
    let mut secs = 0.0;
    let mut prev_beat = 0.0f64;
    let mut bpm = (map[0].bpm as f64).max(1.0);
    for p in &map[1..] {
        let at = p.at.to_f64();
        let span_secs = (at - prev_beat) * 60.0 / bpm;
        if seconds < secs + span_secs {
            return prev_beat + (seconds - secs) * bpm / 60.0;
        }
        secs += span_secs;
        prev_beat = at;
        bpm = (p.bpm as f64).max(1.0);
    }
    prev_beat + (seconds - secs) * bpm / 60.0
}

impl SoundDoc {
    /// Every SoundFont path the document references (each `seq` with
    /// `wave: "sampler"` and a non-empty `sf2`). [`validate`](Self::validate)
    /// is filesystem-free — the core is pure compute — so a *loader* (the CLI,
    /// the Python bindings, a game's asset pipeline) calls this after
    /// validation to check the files exist and fail loud at load time.
    pub fn sf2_paths(&self) -> Vec<&str> {
        fn walk<'doc>(node: &'doc Node, out: &mut Vec<&'doc str>) {
            if let Node::Seq { wave, sf2, .. } = node
                && *wave == SeqWave::Sampler
                && !sf2.sf2.is_empty()
            {
                out.push(sf2.sf2.as_str());
            }
            node.children().for_each(|c| walk(c, out));
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
