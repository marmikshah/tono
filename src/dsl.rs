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
// Seq instrument defaults: ratio 1 + decaying index ≈ an FM piano strike;
// pluck decay 0.996 rings ~1 s in the mid register.
fn default_seq_fm_ratio() -> f32 {
    1.0
}
fn default_seq_fm_index() -> f32 {
    5.0
}
fn default_seq_fm_strike() -> f32 {
    0.2
}
fn default_pluck_decay() -> f32 {
    0.996
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
        /// Instrument used for every note.
        wave: SeqWave,
        /// Duty cycle when `wave` is `square` (may be modulated for PWM).
        #[serde(default = "default_duty")]
        duty: Value,
        /// Modulator frequency ratio when `wave` is `fm` (1 = e-piano/piano,
        /// 3.5 = bell, 14 = tine).
        #[serde(default = "default_seq_fm_ratio")]
        fm_ratio: f32,
        /// FM modulation index at the strike when `wave` is `fm` (brightness;
        /// also scaled by each note's velocity, so louder notes ring brighter).
        #[serde(default = "default_seq_fm_index")]
        fm_index: f32,
        /// Strike decay in seconds when `wave` is `fm`: how fast the index
        /// (brightness) fades after each note's attack. Short = percussive
        /// e-piano, long = sustained bell shimmer.
        #[serde(default = "default_seq_fm_strike")]
        fm_strike: f32,
        /// String feedback decay when `wave` is `pluck`, 0.8..1 (higher rings
        /// longer; low notes naturally ring longer than high ones).
        #[serde(default = "default_pluck_decay")]
        pluck_decay: f32,
        /// Swing, 0..1: every off-beat grid step is delayed by this fraction
        /// of a step (0 = straight, ~0.55 = classic shuffle). Off-beats are
        /// odd steps, so set `steps_per_beat` to the swung subdivision.
        #[serde(default)]
        swing: f32,
        /// Humanize, 0..1: deterministic per-note timing and velocity jitter
        /// (from the doc's seed) so repeats stop sounding machine-perfect.
        /// 0.1–0.25 is a tasteful player; 1 is sloppy.
        #[serde(default)]
        humanize: f32,
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

impl Adsr {
    /// Range-check the envelope shape. `what` prefixes error messages
    /// (e.g. `"env"` ⇒ `"env.a must be >= 0"`).
    fn validate(&self, what: &str) -> Result<(), String> {
        for (n, v) in [("a", self.a), ("d", self.d), ("r", self.r)] {
            if v < 0.0 {
                return Err(format!("{what}.{n} must be >= 0, got {v}"));
            }
        }
        in_unit(&format!("{what}.s"), self.s)?;
        in_unit(&format!("{what}.punch"), self.punch)
    }
}

impl SoundDoc {
    /// Validate ranges and structure beyond what serde already enforces.
    /// Returns a human-readable message the agent can act on.
    pub fn validate(&self) -> Result<(), String> {
        // 600 s covers full songs; the cap exists only to bound render memory.
        if !(self.duration > 0.0 && self.duration <= 600.0) {
            return Err(format!(
                "duration must be in (0, 600] seconds, got {}",
                self.duration
            ));
        }
        if !(8_000..=192_000).contains(&self.sample_rate) {
            return Err(format!(
                "sample_rate must be in [8000, 192000] Hz, got {}",
                self.sample_rate
            ));
        }
        match self.stereo {
            Stereo::Mono => {}
            Stereo::Haas { ms, pan } => {
                if !(0.5..=40.0).contains(&ms) {
                    return Err(format!("stereo.haas.ms must be in [0.5, 40], got {ms}"));
                }
                if !(-1.0..=1.0).contains(&pan) {
                    return Err(format!("stereo.haas.pan must be in [-1, 1], got {pan}"));
                }
            }
            Stereo::Wide { amount } => in_unit("stereo.wide.amount", amount)?,
        }
        if let Some(nz) = &self.normalize {
            if let Some(t) = nz.target_lufs
                && !(-60.0..=0.0).contains(&t)
            {
                return Err(format!(
                    "normalize.target_lufs must be in [-60, 0] LUFS, got {t}"
                ));
            }
            if !(-12.0..=0.0).contains(&nz.ceiling_dbtp) {
                return Err(format!(
                    "normalize.ceiling_dbtp must be in [-12, 0] dBTP, got {}",
                    nz.ceiling_dbtp
                ));
            }
        }
        if let Playback::Loop {
            start_secs,
            end_secs,
            crossfade_secs,
        } = self.playback
        {
            if start_secs < 0.0 || start_secs >= self.duration {
                return Err(format!(
                    "playback.loop.start_secs must be in [0, duration), got {start_secs}"
                ));
            }
            if let Some(end) = end_secs {
                if end <= start_secs {
                    return Err(format!(
                        "playback.loop.end_secs ({end}) must be > start_secs ({start_secs})"
                    ));
                }
                if end > self.duration {
                    return Err(format!(
                        "playback.loop.end_secs ({end}) must be <= duration ({})",
                        self.duration
                    ));
                }
            }
            if crossfade_secs < 0.0 {
                return Err(format!(
                    "playback.loop.crossfade_secs must be >= 0, got {crossfade_secs}"
                ));
            }
        }
        validate_node(&self.root)
    }
}

fn validate_value(v: &Value, what: &str) -> Result<(), String> {
    match v {
        Value::Const(_) => Ok(()),
        Value::Note(s) => note_to_hz(s).map(|_| ()).ok_or_else(|| {
            format!("{what}: '{s}' is not a valid note (e.g. \"A4\", \"C#3\", \"midi:69\")")
        }),
        Value::Modulated(m) => match m {
            Modulator::Slide { secs, .. } => {
                if *secs <= 0.0 {
                    return Err(format!("{what}: slide.secs must be > 0, got {secs}"));
                }
                Ok(())
            }
            Modulator::Lfo { rate, .. } => {
                if *rate <= 0.0 {
                    return Err(format!("{what}: lfo.rate must be > 0, got {rate}"));
                }
                Ok(())
            }
            Modulator::Arp { steps, rate } => {
                if steps.is_empty() {
                    return Err(format!("{what}: arp.steps must be non-empty"));
                }
                if *rate <= 0.0 {
                    return Err(format!("{what}: arp.rate must be > 0, got {rate}"));
                }
                Ok(())
            }
            Modulator::EnvMod { adsr, .. } => adsr.validate(&format!("{what}: env")),
        },
    }
}

fn in_unit(name: &str, v: f32) -> Result<(), String> {
    if !(0.0..=1.0).contains(&v) {
        return Err(format!("{name} must be in [0, 1], got {v}"));
    }
    Ok(())
}

/// EQ gain bound: ±24 dB covers any musical boost/cut; far beyond that the
/// biquad coefficients overflow to inf/NaN and render silent garbage.
fn validate_gain_db(name: &str, v: f32) -> Result<(), String> {
    if !(-24.0..=24.0).contains(&v) {
        return Err(format!("{name} must be in [-24, 24] dB, got {v}"));
    }
    Ok(())
}

/// Validate a `Value` whose constant form must lie in [0, 1] (modulated forms
/// are clamped at render time).
fn validate_unit_value(v: &Value, what: &str) -> Result<(), String> {
    if let Value::Const(c) = v {
        in_unit(what, *c)?;
    }
    validate_value(v, what)
}

fn validate_node(node: &Node) -> Result<(), String> {
    match node {
        Node::Square { freq, duty } => {
            validate_value(freq, "square.freq")?;
            validate_unit_value(duty, "square.duty")
        }
        Node::Triangle { freq } => validate_value(freq, "triangle.freq"),
        Node::Sawtooth { freq } => validate_value(freq, "sawtooth.freq"),
        Node::Sine { freq } => validate_value(freq, "sine.freq"),
        Node::Noise { .. } => Ok(()),
        Node::Fm { freq, ratio, index } => {
            validate_value(freq, "fm.freq")?;
            if *ratio <= 0.0 {
                return Err(format!("fm.ratio must be > 0, got {ratio}"));
            }
            validate_value(index, "fm.index")
        }
        Node::Seq {
            bpm,
            steps_per_beat,
            duty,
            fm_ratio,
            fm_index,
            fm_strike,
            pluck_decay,
            swing,
            humanize,
            env,
            notes,
            ..
        } => {
            if *bpm <= 0.0 {
                return Err(format!("seq.bpm must be > 0, got {bpm}"));
            }
            if *steps_per_beat < 1 {
                return Err("seq.steps_per_beat must be >= 1".into());
            }
            if notes.is_empty() {
                return Err("seq.notes must be non-empty".into());
            }
            validate_unit_value(duty, "seq.duty")?;
            if *fm_ratio <= 0.0 {
                return Err(format!("seq.fm_ratio must be > 0, got {fm_ratio}"));
            }
            if !(0.0..=20.0).contains(fm_index) {
                return Err(format!("seq.fm_index must be in [0, 20], got {fm_index}"));
            }
            if *fm_strike <= 0.0 {
                return Err(format!("seq.fm_strike must be > 0, got {fm_strike}"));
            }
            if !(0.8..1.0).contains(pluck_decay) {
                return Err(format!(
                    "seq.pluck_decay must be in [0.8, 1), got {pluck_decay}"
                ));
            }
            in_unit("seq.swing", *swing)?;
            in_unit("seq.humanize", *humanize)?;
            env.validate("seq.env")?;
            for note in notes {
                if note.len < 1 {
                    return Err("seq note.len must be >= 1".into());
                }
                in_unit("seq note.gain", note.gain)?;
                validate_value(&note.pitch, "seq note.pitch")?;
            }
            Ok(())
        }
        Node::Env { adsr } => adsr.validate("env"),
        Node::Mix { inputs } | Node::Mul { inputs } => {
            if inputs.is_empty() {
                return Err("mix/mul requires at least one input".into());
            }
            inputs.iter().try_for_each(validate_node)
        }
        Node::Chain { stages } => {
            if stages.is_empty() {
                return Err("chain requires at least one stage".into());
            }
            stages.iter().try_for_each(validate_node)
        }
        Node::Lowpass { cutoff, q }
        | Node::Highpass { cutoff, q }
        | Node::Bandpass { cutoff, q }
        | Node::Notch { cutoff, q } => {
            validate_value(cutoff, "filter.cutoff")?;
            if *q <= 0.0 {
                return Err(format!("filter.q must be > 0, got {q}"));
            }
            Ok(())
        }
        Node::Peak { cutoff, q, gain_db } => {
            validate_value(cutoff, "peak.cutoff")?;
            if *q <= 0.0 {
                return Err(format!("peak.q must be > 0, got {q}"));
            }
            validate_gain_db("peak.gain_db", *gain_db)
        }
        Node::Lowshelf { cutoff, gain_db } | Node::Highshelf { cutoff, gain_db } => {
            validate_value(cutoff, "shelf.cutoff")?;
            validate_gain_db("shelf.gain_db", *gain_db)
        }
        Node::Super {
            freq,
            voices,
            detune_cents,
            ..
        } => {
            validate_value(freq, "super.freq")?;
            if !(1..=16).contains(voices) {
                return Err(format!("super.voices must be in [1, 16], got {voices}"));
            }
            if *detune_cents < 0.0 {
                return Err(format!(
                    "super.detune_cents must be >= 0, got {detune_cents}"
                ));
            }
            Ok(())
        }
        Node::Gain { amount } => validate_value(amount, "gain.amount"),
        Node::Bitcrush { bits } => {
            if !(1..=16).contains(bits) {
                return Err(format!("bitcrush.bits must be in [1, 16], got {bits}"));
            }
            Ok(())
        }
        Node::Downsample { factor } => {
            if *factor < 1 {
                return Err("downsample.factor must be >= 1".into());
            }
            Ok(())
        }
        Node::Delay { secs, feedback } => {
            if *secs <= 0.0 {
                return Err(format!("delay.secs must be > 0, got {secs}"));
            }
            in_unit("delay.feedback", *feedback)
        }
        Node::Reverb { room, mix } => {
            in_unit("reverb.room", *room)?;
            in_unit("reverb.mix", *mix)
        }
        Node::Drive { amount, .. } => validate_value(amount, "drive.amount"),
        Node::RingMod { freq } => validate_value(freq, "ringmod.freq"),
        Node::Chorus { rate, depth, mix } => {
            if *rate <= 0.0 {
                return Err(format!("chorus.rate must be > 0, got {rate}"));
            }
            in_unit("chorus.depth", *depth)?;
            in_unit("chorus.mix", *mix)
        }
        Node::Flanger {
            rate,
            depth,
            feedback,
            mix,
        }
        | Node::Phaser {
            rate,
            depth,
            feedback,
            mix,
        } => {
            if *rate <= 0.0 {
                return Err(format!("flanger/phaser.rate must be > 0, got {rate}"));
            }
            in_unit("flanger/phaser.depth", *depth)?;
            in_unit("flanger/phaser.feedback", *feedback)?;
            in_unit("flanger/phaser.mix", *mix)
        }
        Node::Compress {
            ratio,
            attack,
            release,
            ..
        } => {
            if *ratio < 1.0 {
                return Err(format!("compress.ratio must be >= 1, got {ratio}"));
            }
            if *attack < 0.0 || *release < 0.0 {
                return Err("compress.attack/release must be >= 0".into());
            }
            Ok(())
        }
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
    fn note_names_resolve_to_hz() {
        assert_eq!(note_to_hz("A4"), Some(440.0));
        assert_eq!(note_to_hz("midi:69"), Some(440.0));
        assert_eq!(note_to_hz("m69"), Some(440.0));
        // C#3 = midi 49 ≈ 138.59 Hz; Gb5 = midi 78 ≈ 739.99 Hz.
        assert!((note_to_hz("C#3").unwrap() - 138.591).abs() < 0.01);
        assert!((note_to_hz("Gb5").unwrap() - 739.989).abs() < 0.01);
        // Octave defaults to 4; accidentals stack; case-insensitive letter.
        assert_eq!(note_to_hz("A"), Some(440.0));
        assert_eq!(note_to_hz("a4"), Some(440.0));
        assert!((note_to_hz("F#-1").unwrap() - 11.562).abs() < 0.01);
        // Garbage stays unparsed.
        assert_eq!(note_to_hz(""), None);
        assert_eq!(note_to_hz("H4"), None);
        assert_eq!(note_to_hz("A4x"), None);
    }

    #[test]
    fn processors_are_processors_sources_are_not() {
        let p: Node = serde_json::from_str(r#"{ "type": "reverb", "room": 0.5 }"#).unwrap();
        assert!(p.is_processor());
        let s: Node = serde_json::from_str(r#"{ "type": "sine", "freq": 440 }"#).unwrap();
        assert!(!s.is_processor());
    }

    fn doc(json: &str) -> SoundDoc {
        serde_json::from_str(json).expect("deserialize")
    }

    #[test]
    fn validate_accepts_a_sane_doc() {
        let d = doc(
            r#"{ "name": "zap", "duration": 0.2, "root": { "type": "mul", "inputs": [
                { "type": "square", "freq": { "slide": { "from": 880, "to": 180, "secs": 0.18 } } },
                { "type": "env", "d": 0.18, "punch": 0.3 }
            ] } }"#,
        );
        assert_eq!(d.validate(), Ok(()));
    }

    #[test]
    fn validate_rejects_out_of_range_metadata() {
        let d = doc(r#"{ "name": "n", "duration": 0, "root": { "type": "noise" } }"#);
        assert!(d.validate().unwrap_err().contains("duration"));
        let d = doc(r#"{ "name": "n", "sample_rate": 1000, "root": { "type": "noise" } }"#);
        assert!(d.validate().unwrap_err().contains("sample_rate"));
        let d = doc(
            r#"{ "name": "n", "normalize": { "target_lufs": 5 }, "root": { "type": "noise" } }"#,
        );
        assert!(d.validate().unwrap_err().contains("target_lufs"));
    }

    #[test]
    fn validate_rejects_bad_loop_region() {
        let d = doc(r#"{ "name": "n", "duration": 1,
                 "playback": { "mode": "loop", "start_secs": 0.8, "end_secs": 0.5 },
                 "root": { "type": "noise" } }"#);
        assert!(d.validate().unwrap_err().contains("end_secs"));
    }

    #[test]
    fn validate_rejects_unit_range_violations() {
        let d = doc(r#"{ "name": "n", "root": { "type": "chain", "stages": [
                { "type": "noise" }, { "type": "reverb", "mix": 1.5 }
            ] } }"#);
        assert!(d.validate().unwrap_err().contains("reverb.mix"));
        let d = doc(r#"{ "name": "n", "root": { "type": "env", "s": 2 } }"#);
        assert!(d.validate().unwrap_err().contains("env.s"));
    }

    #[test]
    fn validate_rejects_extreme_eq_gain() {
        // Beyond ±24 dB the biquad coefficients blow up to inf/NaN.
        let d = doc(r#"{ "name": "n", "root": { "type": "chain", "stages": [
                { "type": "noise" }, { "type": "peak", "cutoff": 1000, "gain_db": 2000 }
            ] } }"#);
        assert!(d.validate().unwrap_err().contains("peak.gain_db"));
        let d = doc(r#"{ "name": "n", "root": { "type": "chain", "stages": [
                { "type": "noise" }, { "type": "lowshelf", "cutoff": 200, "gain_db": -100 }
            ] } }"#);
        assert!(d.validate().unwrap_err().contains("shelf.gain_db"));
    }

    #[test]
    fn validate_rejects_empty_combinators_and_bad_notes() {
        let d = doc(r#"{ "name": "n", "root": { "type": "mix", "inputs": [] } }"#);
        assert!(d.validate().unwrap_err().contains("mix/mul"));
        let d = doc(r#"{ "name": "n", "root": { "type": "sine", "freq": "H9" } }"#);
        assert!(d.validate().unwrap_err().contains("not a valid note"));
    }
}
