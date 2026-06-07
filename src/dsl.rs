//! The Sonarium synthesis-graph DSL.
//!
//! A [`SoundDoc`] is the canonical, declarative source of a sound. The AI agent
//! authors one of these; the renderer turns it into samples. Everything here is
//! `serde`-deserializable (the MCP wire format is JSON) and `JsonSchema`-
//! describable so the agent can self-correct against the schema.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Current DSL schema version. Stored on every doc so old graphs stay loadable
/// as the vocabulary evolves.
pub const SCHEMA_VERSION: u32 = 1;

// Serde `default = "..."` requires free functions. Values with non-obvious
// origins: q 0.707 is Butterworth (maximally flat), haas 12 ms sits in the
// precedence-effect sweet spot, ceiling −1 dBTP is the common streaming-safe
// true-peak ceiling.
fn default_version() -> u32 {
    SCHEMA_VERSION
}
fn default_sample_rate() -> u32 {
    44_100
}
fn default_duration() -> f32 {
    0.3
}
fn default_duty() -> Value {
    Value::Const(0.5)
}
fn default_q() -> f32 {
    0.707
}
fn default_gain() -> f32 {
    1.0
}
fn default_steps_per_beat() -> u32 {
    4
}
// Shared by chorus / flanger / phaser ("mod fx").
fn default_mod_depth() -> f32 {
    0.5
}
fn default_mod_mix() -> f32 {
    0.5
}
fn default_chorus_rate() -> f32 {
    1.5
}
fn default_flanger_rate() -> f32 {
    0.25
}
fn default_flanger_feedback() -> f32 {
    0.5
}
fn default_phaser_rate() -> f32 {
    0.4
}
fn default_phaser_feedback() -> f32 {
    0.3
}
fn default_comp_attack() -> f32 {
    0.005
}
fn default_comp_release() -> f32 {
    0.08
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
fn default_voices() -> u32 {
    7
}
fn default_detune() -> f32 {
    15.0
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
    /// DSL schema version. Defaults to the current version.
    #[serde(default = "default_version")]
    pub version: u32,
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

/// How the rendered sound is meant to be played back.
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
}

/// A node in the synthesis graph. Every node evaluates to a mono signal.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum Node {
    // --- Sources (output audio in [-1, 1]) ---
    /// Square/pulse wave with variable duty cycle. `duty` may be a modulator
    /// (e.g. an `lfo`) for PWM — the classic moving chiptune lead.
    Square {
        /// Frequency in Hz.
        freq: Value,
        /// Fraction of the period spent "high", 0..1 (0.5 = symmetric square).
        #[serde(default = "default_duty")]
        duty: Value,
    },
    /// Triangle wave.
    Triangle {
        /// Frequency in Hz.
        freq: Value,
    },
    /// Sawtooth wave.
    Sawtooth {
        /// Frequency in Hz.
        freq: Value,
    },
    /// Sine wave.
    Sine {
        /// Frequency in Hz.
        freq: Value,
    },
    /// Noise source (percussion / explosion / texture). White by default;
    /// `pink` is warmer (−3 dB/oct, good for wind/rumble), `brown` darker still.
    Noise {
        /// Spectral colour of the noise.
        #[serde(default)]
        color: NoiseColor,
    },
    /// Two-operator FM: a carrier at `freq` is phase-modulated by an operator
    /// at `freq * ratio` with modulation `index`. Bells, e-piano, metallic
    /// basses. Slide `index` down for a struck/plucked attack.
    Fm {
        /// Carrier frequency in Hz.
        freq: Value,
        /// Modulator frequency as a multiple of the carrier (e.g. 2.0, 3.5).
        ratio: f32,
        /// Modulation index (depth). Higher ⇒ brighter / more sidebands.
        index: Value,
    },
    /// Unison "super" oscillator: `voices` detuned copies of a band-limited
    /// saw/square, summed for a fat, wide lead or pad (the supersaw). Detune is
    /// spread symmetrically across `detune_cents`.
    Super {
        /// Oscillator shape for every voice.
        #[serde(default)]
        wave: SuperWave,
        /// Centre frequency in Hz (modulatable).
        freq: Value,
        /// Number of detuned voices (1..=16).
        #[serde(default = "default_voices")]
        voices: u32,
        /// Total detune spread in cents across all voices.
        #[serde(default = "default_detune")]
        detune_cents: f32,
    },
    /// A note sequencer: plays `notes` on a tempo grid, each with its own pitch,
    /// length, and a shared per-note ADSR. This is how you write real melodies,
    /// basslines, and drum patterns (rests = gaps between notes).
    Seq {
        /// Tempo in beats per minute.
        bpm: f32,
        /// Grid resolution: steps per beat (4 = sixteenth notes).
        #[serde(default = "default_steps_per_beat")]
        steps_per_beat: u32,
        /// Oscillator used for every note.
        wave: SeqWave,
        /// Duty cycle when `wave` is `square` (may be modulated for PWM).
        #[serde(default = "default_duty")]
        duty: Value,
        /// Per-note amplitude envelope.
        env: Adsr,
        /// The notes to play.
        notes: Vec<SeqNote>,
    },

    // --- Envelope (outputs a 0..1 control signal) ---
    /// ADSR amplitude envelope with an sfxr-style `punch` transient.
    Env {
        /// Envelope shape.
        #[serde(flatten)]
        adsr: Adsr,
    },

    // --- Combinators ---
    /// Sum (layer) all inputs.
    Mix {
        /// Branches to add together.
        inputs: Vec<Node>,
    },
    /// Multiply all inputs (typically `source × envelope`).
    Mul {
        /// Branches to multiply together.
        inputs: Vec<Node>,
    },
    /// Serial pipe: stage 0 is a source, each later stage processes the prior output.
    Chain {
        /// Ordered processing stages.
        stages: Vec<Node>,
    },

    // --- Processors (transform the preceding signal in a chain) ---
    /// Resonant low-pass filter.
    Lowpass {
        /// Cutoff frequency in Hz.
        cutoff: Value,
        /// Resonance / quality factor.
        #[serde(default = "default_q")]
        q: f32,
    },
    /// Resonant high-pass filter.
    Highpass {
        /// Cutoff frequency in Hz.
        cutoff: Value,
        /// Resonance / quality factor.
        #[serde(default = "default_q")]
        q: f32,
    },
    /// Band-pass filter.
    Bandpass {
        /// Center frequency in Hz.
        cutoff: Value,
        /// Resonance / quality factor.
        #[serde(default = "default_q")]
        q: f32,
    },
    /// Notch (band-reject) filter: removes a narrow band around `cutoff` (hum /
    /// resonance removal).
    Notch {
        /// Center frequency in Hz.
        cutoff: Value,
        /// Quality factor (higher ⇒ narrower notch).
        #[serde(default = "default_q")]
        q: f32,
    },
    /// Peaking EQ: boost or cut a band around `cutoff` by `gain_db` (surgical
    /// tone shaping — act on the brightness/centroid the analyzer reports).
    Peak {
        /// Center frequency in Hz.
        cutoff: Value,
        /// Quality factor (bandwidth).
        #[serde(default = "default_q")]
        q: f32,
        /// Gain in dB (positive boosts, negative cuts).
        #[serde(default)]
        gain_db: f32,
    },
    /// Low shelf: boost/cut everything below `cutoff` by `gain_db` (add weight or
    /// thin out lows).
    Lowshelf {
        /// Shelf corner frequency in Hz.
        cutoff: Value,
        /// Gain in dB.
        #[serde(default)]
        gain_db: f32,
    },
    /// High shelf: boost/cut everything above `cutoff` by `gain_db` (air / de-ess).
    Highshelf {
        /// Shelf corner frequency in Hz.
        cutoff: Value,
        /// Gain in dB.
        #[serde(default)]
        gain_db: f32,
    },
    /// Scale the signal by a (possibly modulated) factor.
    Gain {
        /// Multiplier.
        amount: Value,
    },
    /// Quantize amplitude to `bits` of resolution for crunch.
    Bitcrush {
        /// Bit depth, 1..16.
        bits: u8,
    },
    /// Sample-rate reduction by an integer `factor` for lo-fi grit.
    Downsample {
        /// Hold each sample for this many samples.
        factor: u32,
    },
    /// Feedback delay (echo / comb).
    Delay {
        /// Delay time in seconds.
        secs: f32,
        /// Feedback amount, 0..1.
        #[serde(default)]
        feedback: f32,
    },
    /// Schroeder-style reverb.
    Reverb {
        /// Room size, 0..1 (larger ⇒ longer tail).
        #[serde(default)]
        room: f32,
        /// Dry/wet mix, 0..1.
        #[serde(default)]
        mix: f32,
    },
    /// Waveshaper for saturation / distortion. `amount` is pre-gain; `shape`
    /// chooses the curve (warm `tanh`, aggressive `hard` clip, or `fold`back).
    Drive {
        /// Drive amount (pre-gain into the shaper).
        amount: Value,
        /// Distortion curve.
        #[serde(default)]
        shape: DriveShape,
    },
    /// Ring modulation: multiply the signal by a sine carrier at `freq`. Metallic,
    /// clangorous, robotic textures.
    RingMod {
        /// Carrier frequency in Hz.
        freq: Value,
    },
    /// Flanger: a very short modulated delay with feedback — the classic jet
    /// sweep / metallic whoosh. Stronger and more resonant than chorus.
    Flanger {
        /// LFO rate in Hz.
        #[serde(default = "default_flanger_rate")]
        rate: f32,
        /// Modulation depth, 0..1.
        #[serde(default = "default_mod_depth")]
        depth: f32,
        /// Feedback amount, 0..1 (more ⇒ more resonant).
        #[serde(default = "default_flanger_feedback")]
        feedback: f32,
        /// Dry/wet mix, 0..1.
        #[serde(default = "default_mod_mix")]
        mix: f32,
    },
    /// Phaser: swept all-pass notches — a hollow, swooshing movement for pads,
    /// lasers, and sci-fi textures.
    Phaser {
        /// LFO sweep rate in Hz.
        #[serde(default = "default_phaser_rate")]
        rate: f32,
        /// Sweep depth, 0..1.
        #[serde(default = "default_mod_depth")]
        depth: f32,
        /// Feedback amount, 0..1.
        #[serde(default = "default_phaser_feedback")]
        feedback: f32,
        /// Dry/wet mix, 0..1.
        #[serde(default = "default_mod_mix")]
        mix: f32,
    },
    /// Chorus: a short modulated delay mixed with the dry signal for thickening
    /// and width.
    Chorus {
        /// LFO rate in Hz.
        #[serde(default = "default_chorus_rate")]
        rate: f32,
        /// Modulation depth, 0..1.
        #[serde(default = "default_mod_depth")]
        depth: f32,
        /// Dry/wet mix, 0..1.
        #[serde(default = "default_mod_mix")]
        mix: f32,
    },
    /// Dynamic-range compressor: tames peaks above `threshold` (dBFS) by `ratio`,
    /// with `attack`/`release` ballistics, then applies `makeup` gain (dB). The
    /// glue behind loud, punchy game audio.
    Compress {
        /// Threshold in dBFS (e.g. -18).
        threshold: f32,
        /// Compression ratio (e.g. 4 = 4:1).
        ratio: f32,
        /// Attack time in seconds.
        #[serde(default = "default_comp_attack")]
        attack: f32,
        /// Release time in seconds.
        #[serde(default = "default_comp_release")]
        release: f32,
        /// Make-up gain in dB.
        #[serde(default)]
        makeup: f32,
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
}

/// An ADSR amplitude envelope. One shape, used in three places: the [`Node::Env`]
/// amplitude envelope, the per-note envelope of a [`Node::Seq`], and (with a
/// `from`/`to` range) the [`Modulator::EnvMod`] parameter envelope.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, JsonSchema)]
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

impl Node {
    /// True if this node only makes sense as a non-first stage of a `chain`
    /// (it transforms an incoming signal rather than generating one).
    pub fn is_processor(&self) -> bool {
        matches!(
            self,
            Node::Lowpass { .. }
                | Node::Highpass { .. }
                | Node::Bandpass { .. }
                | Node::Notch { .. }
                | Node::Peak { .. }
                | Node::Lowshelf { .. }
                | Node::Highshelf { .. }
                | Node::Gain { .. }
                | Node::Bitcrush { .. }
                | Node::Downsample { .. }
                | Node::Delay { .. }
                | Node::Reverb { .. }
                | Node::Drive { .. }
                | Node::RingMod { .. }
                | Node::Chorus { .. }
                | Node::Flanger { .. }
                | Node::Phaser { .. }
                | Node::Compress { .. }
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn roundtrip(json: &str) -> serde_json::Value {
        let doc: SoundDoc = serde_json::from_str(json).expect("deserialize");
        serde_json::to_value(&doc).expect("serialize")
    }

    #[test]
    fn doc_defaults_fill_in() {
        let doc: SoundDoc =
            serde_json::from_str(r#"{ "name": "beep", "root": { "type": "sine", "freq": 440 } }"#)
                .unwrap();
        assert_eq!(doc.duration, 0.3);
        assert_eq!(doc.sample_rate, 44_100);
        assert_eq!(doc.version, SCHEMA_VERSION);
        assert!(matches!(doc.stereo, Stereo::Mono));
        assert!(matches!(doc.playback, Playback::OneShot));
    }

    #[test]
    fn node_tag_is_type_lowercase() {
        let v = roundtrip(r#"{ "name": "n", "root": { "type": "ringmod", "freq": 100 } }"#);
        assert_eq!(v["root"]["type"], "ringmod");
    }

    #[test]
    fn env_flattens_adsr_fields_inline() {
        // The wire shape keeps a/d/s/r/punch inline on the env node — the
        // internal Adsr struct must stay invisible to the JSON.
        let v = roundtrip(
            r#"{ "name": "n", "root": { "type": "env", "a": 0.01, "d": 0.2, "punch": 0.3 } }"#,
        );
        assert_eq!(v["root"]["a"], 0.01f32 as f64);
        assert_eq!(v["root"]["punch"], 0.3f32 as f64);
        assert!(v["root"].get("adsr").is_none());
    }

    #[test]
    fn value_untagged_forms() {
        let doc: SoundDoc = serde_json::from_str(
            r#"{ "name": "n", "root": { "type": "mix", "inputs": [
                { "type": "sine", "freq": 440 },
                { "type": "sine", "freq": "A4" },
                { "type": "sine", "freq": { "slide": { "from": 880, "to": 180, "secs": 0.2 } } },
                { "type": "sine", "freq": { "lfo": { "rate": 5, "depth": 10, "center": 440 } } },
                { "type": "sine", "freq": { "arp": { "steps": [523, 659], "rate": 12 } } },
                { "type": "sine", "freq": { "env": { "a": 0.1, "from": 100, "to": 800 } } }
            ] } }"#,
        )
        .unwrap();
        let Node::Mix { inputs } = &doc.root else {
            panic!("expected mix");
        };
        assert!(matches!(
            &inputs[0],
            Node::Sine {
                freq: Value::Const(f)
            } if *f == 440.0
        ));
        assert!(matches!(&inputs[1], Node::Sine { freq: Value::Note(s) } if s == "A4"));
        assert!(matches!(
            &inputs[2],
            Node::Sine {
                freq: Value::Modulated(Modulator::Slide {
                    curve: Curve::Lin,
                    ..
                })
            }
        ));
        assert!(matches!(
            &inputs[5],
            Node::Sine {
                freq: Value::Modulated(Modulator::EnvMod { adsr, .. })
            } if adsr.a == 0.1
        ));
    }

    #[test]
    fn playback_loop_tag() {
        let v = roundtrip(
            r#"{ "name": "n", "playback": { "mode": "loop", "crossfade_secs": 0.25 },
                 "root": { "type": "noise" } }"#,
        );
        assert_eq!(v["playback"]["mode"], "loop");
        assert_eq!(v["playback"]["crossfade_secs"], 0.25f32 as f64);
    }

    #[test]
    fn stereo_modes() {
        let v = roundtrip(
            r#"{ "name": "n", "stereo": { "mode": "haas", "pan": -1 },
                 "root": { "type": "noise" } }"#,
        );
        assert_eq!(v["stereo"]["mode"], "haas");
        assert_eq!(v["stereo"]["ms"], 12.0); // default filled in
    }

    #[test]
    fn processors_are_processors_sources_are_not() {
        let p: Node = serde_json::from_str(r#"{ "type": "reverb", "room": 0.5 }"#).unwrap();
        assert!(p.is_processor());
        let s: Node = serde_json::from_str(r#"{ "type": "sine", "freq": 440 }"#).unwrap();
        assert!(!s.is_processor());
    }
}
