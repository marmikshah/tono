//! The Tono synthesis-graph DSL.
//!
//! A [`SoundDoc`] is the canonical, declarative source of a sound. The AI agent
//! authors one of these; the renderer turns it into samples. Everything here is
//! `serde`-deserializable (the on-disk / wire format is JSON) and `JsonSchema`-
//! describable so a tool can self-correct against the schema.

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
pub const ENGINE_VERSION: u32 = 2;

// Serde `default = "..."` requires free functions. Values with non-obvious
// origins: q 0.707 is Butterworth (maximally flat), haas 12 ms sits in the
// precedence-effect sweet spot, ceiling −1 dBTP is the common streaming-safe
// true-peak ceiling.
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
fn default_duck_amount() -> f32 {
    0.8
}
fn default_duck_attack() -> f32 {
    0.005
}
fn default_duck_release() -> f32 {
    0.25
}
fn default_mode_decay() -> f32 {
    0.4
}
fn default_modal_mix() -> f32 {
    1.0
}
fn default_impact_hardness() -> f32 {
    0.5
}
fn default_dust_decay() -> f32 {
    0.02
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
        /// Path to a SoundFont (.sf2) file when `wave` is `sampler`.
        #[serde(default)]
        sf2: String,
        /// General MIDI program number (0..=127) when `wave` is `sampler`.
        #[serde(default)]
        sf2_preset: u32,
        /// SoundFont bank when `wave` is `sampler` (0 = melodic, 128 = the
        /// percussion bank / GM drum map).
        #[serde(default)]
        sf2_bank: u32,
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

    /// Impact exciter: a short excitation burst that models the contact force
    /// of a strike — a single raised-cosine force pulse whose width is set by
    /// `hardness` (hard = brief, bright click; soft = wider, duller thud),
    /// scaled by `velocity`. On its own it is a faint tick; its job is to
    /// *excite* a resonant body — put it before a `modal` bank (or any
    /// resonant filter) in a `chain`: `chain[ impact, modal ]` is a struck
    /// object. The harder the strike, the more high modes it lights up.
    Impact {
        /// Strike hardness, 0..1 (1 = hardest / brightest / shortest contact).
        #[serde(default = "default_impact_hardness")]
        hardness: f32,
        /// Strike velocity / level, 0..1.
        #[serde(default = "default_gain")]
        velocity: f32,
    },
    /// Sparse stochastic impulses — a Poisson click train. `density` events per
    /// second fire at random times with random ± amplitude, each decaying over
    /// `decay` seconds (0 = bare single-sample impulses). The grain generator
    /// behind crackle textures: fire, rain, geiger ticks, sparks, debris. Feed
    /// it through a `bandpass`/`highpass` (for tone) or a `modal` (for pitched
    /// debris). Its randomness draws from the layer's deterministic stream, so
    /// like `noise` it is edit-stable within its own mixer layer.
    Dust {
        /// Mean events per second.
        density: f32,
        /// Per-grain decay time in seconds (0 = single-sample impulses).
        #[serde(default = "default_dust_decay")]
        decay: f32,
    },

    // --- Envelope (outputs a 0..1 control signal) ---
    /// ADSR amplitude envelope with an sfxr-style `punch` transient.
    Env {
        /// Envelope shape.
        #[serde(flatten)]
        adsr: Adsr,
    },

    // --- Combinators ---
    /// The mixing console — only valid as the document root. Each track is a
    /// mono graph placed on the stereo stage with its own pan and gain
    /// (sampler tracks keep their native stereo); `master` is a processor
    /// chain applied to the stereo bus (compressor glue, reverb — the reverb
    /// runs with decorrelated left/right tails). This is how multi-
    /// instrument music gets a real stereo image instead of a mono sum.
    Tracks {
        /// The mixer channels.
        tracks: Vec<Track>,
        /// Processors applied to the stereo master bus, in order.
        #[serde(default)]
        master: Vec<Node>,
    },
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
    /// Modal resonator bank: a set of damped sinusoidal partials (`modes`)
    /// excited by the incoming signal — a struck/resonant object's *body*.
    /// Bells, glass, metal bars, wood, ceramic, coins, and the resonant ping
    /// of UI/impact sounds, none of which the oscillators can voice cleanly.
    /// Each mode is one 2-pole resonator (a constant-peak-gain bandpass), so a
    /// bank is N parallel resonators — cheap, stable, deterministic. Use it as
    /// a chain stage after an excitation: `chain[ impact, modal ]`. The
    /// excitation's brightness lights the modes; the modes' frequencies and
    /// decays define the timbre. Author modes explicitly — the cookbook lists
    /// frequency/decay tables for common materials to copy and tune.
    Modal {
        /// The resonant partials (1..=64). Each is a damped sine.
        modes: Vec<Mode>,
        /// Wet/dry mix, 0..1 (1 = pure resonance; lower keeps some of the raw
        /// excitation transient for extra attack click).
        #[serde(default = "default_modal_mix")]
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
        /// Antiderivative anti-aliasing. The waveshaper's harmonics fold back
        /// as inharmonic alias dirt at the base rate; ADAA suppresses that for
        /// a clean, hi-fi distortion. Honoured only when the document's
        /// `engine` is ≥ 1 (so legacy documents stay bit-exact); within an
        /// engine-1 document it is on by default — set `false` to hear the raw
        /// aliasing curve. Omitted ⇒ follow the engine.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        aa: Option<bool>,
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
    /// Sidechain duck: gain-reduces the chained signal whenever `trigger` is
    /// loud — the pumping that glues a bass or pad to the kick. The trigger
    /// is rendered silently (it only steers the gain); chain the audible
    /// kick separately in the mix.
    Duck {
        /// The signal whose loudness drives the ducking (e.g. the kick seq).
        trigger: Box<Node>,
        /// Duck depth, 0..1 (1 = fully silent at the trigger's peak).
        #[serde(default = "default_duck_amount")]
        amount: f32,
        /// Gain-reduction attack in seconds.
        #[serde(default = "default_duck_attack")]
        attack: f32,
        /// Recovery time in seconds (the "pump" length).
        #[serde(default = "default_duck_release")]
        release: f32,
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
    /// SoundFont sampler: plays the notes through real recorded instruments
    /// from an `.sf2` file (set the seq's `sf2` path and `sf2_preset` — the
    /// General MIDI program number, e.g. 0 grand piano, 32 acoustic bass,
    /// 48 strings; `sf2_bank: 128` selects the percussion bank, where notes
    /// follow the GM drum map). The biggest realism jump available: this is
    /// how DAWs sound real.
    Sampler,
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

/// One mixer channel in a [`Node::Tracks`] root.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct Track {
    /// Stable layer id — a short slug like `"kick"` or `"tail"`, unique within
    /// the document. This is how edits address the track by id, so it never
    /// shifts when sibling layers are added or
    /// removed (unlike an array index). Omitted ids are backfilled
    /// deterministically (`layer_<position>`) on the next build.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    /// The track's signal graph (usually a `seq` or a `chain`).
    pub node: Node,
    /// Stereo position, −1 (hard left) .. 1 (hard right). Equal-power law.
    #[serde(default)]
    pub pan: f32,
    /// Channel fader, 0..2 (1 = unity).
    #[serde(default = "default_gain")]
    pub gain: f32,
    /// Start offset in seconds: the rendered layer is shifted this far right
    /// on the bus (the transient + body + tail recipe). The render keeps its
    /// full length and the shifted tail is truncated at the document edge.
    #[serde(default)]
    pub at: f32,
    /// Muted layers stay in the document but are left off the bus. This is
    /// rendered state, not a monitoring convenience — exports ship without
    /// muted layers.
    #[serde(default)]
    pub mute: bool,
    /// Song-time automation lanes for this track's `gain` / `pan` (volume rides,
    /// pan moves across sections). Empty ⇒ the static `gain`/`pan` apply and the
    /// render is byte-identical to a document without this field. A lane's value
    /// overrides the static one over time; per-node modulators still cover the
    /// node level (this is the track/song level).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub automation: Vec<AutoLane>,
}

/// What a track automation lane controls.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum AutoTarget {
    /// The track's channel fader (0..2).
    Gain,
    /// The track's stereo position (−1..1).
    Pan,
}

/// One breakpoint in an automation lane: value `v` at song time `t` seconds.
/// Between breakpoints the value is linearly interpolated; before the first /
/// after the last it holds flat.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct AutoPoint {
    /// Song time in seconds.
    pub t: f32,
    /// Target value at this time.
    pub v: f32,
}

/// A track automation lane: a `target` driven by a list of breakpoints.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct AutoLane {
    /// What this lane controls.
    pub target: AutoTarget,
    /// Breakpoints over song time.
    pub points: Vec<AutoPoint>,
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
                | Node::Modal { .. }
                | Node::Drive { .. }
                | Node::RingMod { .. }
                | Node::Chorus { .. }
                | Node::Flanger { .. }
                | Node::Phaser { .. }
                | Node::Compress { .. }
                | Node::Duck { .. }
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

    /// Validate ranges and structure beyond what serde already enforces.
    /// Returns a human-readable message the agent can act on.
    pub fn validate(&self) -> Result<(), String> {
        let v = self.effective_version();
        if v == 0 || v > SCHEMA_VERSION {
            return Err(format!(
                "version must be in [1, {SCHEMA_VERSION}], got {v} — a document from a newer \
                 tono cannot render correctly here; upgrade tono"
            ));
        }
        let e = self.effective_engine();
        if e > ENGINE_VERSION {
            return Err(format!(
                "engine must be in [0, {ENGINE_VERSION}], got {e} — a document authored against \
                 a newer DSP kernel cannot render correctly here; upgrade tono"
            ));
        }
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
        if let Node::Tracks { tracks, master } = &self.root {
            if tracks.is_empty() {
                return Err("tracks must be non-empty".into());
            }
            // A mixer document builds its stereo image from per-layer pan; a
            // doc-level Haas/Wide treatment would be silently dropped by the
            // renderer. v1 documents keep the historical silent-ignore so old
            // libraries still load.
            if self.effective_version() >= 2 && !matches!(self.stereo, Stereo::Mono) {
                return Err(
                    "a tracks document builds its stereo image from per-layer pan — remove the \
                     doc-level stereo treatment (set stereo mode 'mono') and pan the layers \
                     instead"
                        .into(),
                );
            }
            let mut seen_ids = std::collections::HashSet::new();
            let mut seen_streams = std::collections::HashMap::new();
            for (i, t) in tracks.iter().enumerate() {
                // Errors name the layer by id when it has one — that is the
                // address the agent used.
                let who = match &t.id {
                    Some(id) => format!("layer '{id}'"),
                    None => format!("tracks[{i}]"),
                };
                if let Some(id) = &t.id {
                    if id.is_empty()
                        || !id
                            .chars()
                            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
                    {
                        return Err(format!(
                            "{who}: layer ids are short slugs (a-z, 0-9, _), got '{id}'"
                        ));
                    }
                    if id == "master" {
                        return Err(
                            "'master' is reserved for the master chain; pick another layer id"
                                .into(),
                        );
                    }
                    if !seen_ids.insert(id.clone()) {
                        return Err(format!("duplicate layer id '{id}' — ids must be unique"));
                    }
                    // Stream keys must be collision-free or two layers would
                    // silently share one noise stream (and u64::MAX is the
                    // master bus's stream).
                    let key = crate::dsp::layer_stream_key(id);
                    if key == u64::MAX {
                        return Err(format!(
                            "{who}: this id collides with the master bus's RNG stream — rename \
                             the layer"
                        ));
                    }
                    if let Some(other) = seen_streams.insert(key, id.clone()) {
                        return Err(format!(
                            "layer ids '{other}' and '{id}' hash to the same RNG stream — \
                             rename one of them"
                        ));
                    }
                }
                if !(-1.0..=1.0).contains(&t.pan) {
                    return Err(format!("{who}: pan must be in [-1, 1], got {}", t.pan));
                }
                if !(0.0..=2.0).contains(&t.gain) {
                    return Err(format!("{who}: gain must be in [0, 2], got {}", t.gain));
                }
                if !(0.0..self.duration).contains(&t.at) {
                    return Err(format!(
                        "{who}: at must be in [0, duration {}), got {} — the layer would be \
                         entirely outside the render window",
                        self.duration, t.at
                    ));
                }
                if contains_tracks(&t.node) {
                    return Err("tracks cannot nest inside a track".into());
                }
                validate_node(&t.node)?;
            }
            for (i, m) in master.iter().enumerate() {
                if !m.is_processor() {
                    return Err(format!(
                        "master[{i}] must be a processor (filter/eq/dynamics/fx)"
                    ));
                }
                validate_node(m)?;
            }
            return Ok(());
        }
        if contains_tracks(&self.root) {
            return Err("tracks is the mixing console: it must be the document's root node".into());
        }
        validate_node(&self.root)
    }
}

/// True if a `tracks` node appears anywhere in this subtree.
fn contains_tracks(node: &Node) -> bool {
    match node {
        Node::Tracks { .. } => true,
        Node::Mix { inputs } | Node::Mul { inputs } => inputs.iter().any(contains_tracks),
        Node::Chain { stages } => stages.iter().any(contains_tracks),
        Node::Duck { trigger, .. } => contains_tracks(trigger),
        _ => false,
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
            Modulator::Rand { rate, .. } => {
                if *rate <= 0.0 {
                    return Err(format!("{what}: rand.rate must be > 0, got {rate}"));
                }
                Ok(())
            }
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
        Node::Impact { hardness, velocity } => {
            in_unit("impact.hardness", *hardness)?;
            in_unit("impact.velocity", *velocity)
        }
        Node::Dust { density, decay } => {
            if *density <= 0.0 {
                return Err(format!("dust.density must be > 0, got {density}"));
            }
            if *decay < 0.0 {
                return Err(format!("dust.decay must be >= 0, got {decay}"));
            }
            Ok(())
        }
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
            sf2,
            sf2_preset,
            wave,
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
            if *wave == SeqWave::Sampler {
                if sf2.is_empty() {
                    return Err(
                        "seq.sf2 must point at a SoundFont (.sf2) file when wave is 'sampler'"
                            .into(),
                    );
                }
                if !std::path::Path::new(sf2).exists() {
                    return Err(format!("seq.sf2: no such file '{sf2}'"));
                }
                if *sf2_preset > 127 {
                    return Err(format!(
                        "seq.sf2_preset must be in 0..=127, got {sf2_preset}"
                    ));
                }
            }
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
        Node::Env { adsr } => {
            adsr.validate("env")?;
            // An all-zero envelope is always silent — never intended. It's also
            // the tell-tale of the flatten footgun: the env's a/d/s/r are inlined
            // (`{"type":"env","a":..,"d":..}`), so wrapping them in an `"adsr"`
            // object silently drops them all to 0. Reject it with that hint.
            if adsr.a == 0.0 && adsr.d == 0.0 && adsr.s == 0.0 && adsr.r == 0.0 {
                return Err("env is silent — a/d/s/r are all 0. The envelope fields \
                    are inlined on the node (e.g. {\"type\":\"env\",\"a\":0.01,\"d\":0.1,\
                    \"s\":0.7,\"r\":0.2}); don't nest them under \"adsr\""
                    .into());
            }
            Ok(())
        }
        // Nested mixers are rejected earlier; this guards direct calls.
        Node::Tracks { .. } => Err("tracks must be the document's root node".into()),
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
        Node::Modal { modes, mix } => {
            if modes.is_empty() {
                return Err("modal.modes must be non-empty".into());
            }
            if modes.len() > 64 {
                return Err(format!(
                    "modal.modes must have at most 64 modes, got {}",
                    modes.len()
                ));
            }
            for (i, m) in modes.iter().enumerate() {
                if m.freq <= 0.0 {
                    return Err(format!("modal.modes[{i}].freq must be > 0, got {}", m.freq));
                }
                if m.decay <= 0.0 {
                    return Err(format!(
                        "modal.modes[{i}].decay must be > 0, got {}",
                        m.decay
                    ));
                }
                in_unit(&format!("modal.modes[{i}].gain"), m.gain)?;
            }
            in_unit("modal.mix", *mix)
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
        Node::Duck {
            trigger,
            amount,
            attack,
            release,
        } => {
            in_unit("duck.amount", *amount)?;
            if *attack < 0.0 || *release < 0.0 {
                return Err("duck.attack/release must be >= 0".into());
            }
            validate_node(trigger)
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

    fn doc_with_root(root: &str) -> SoundDoc {
        serde_json::from_str(&format!(
            r#"{{ "name": "t", "duration": 0.2, "engine": 1, "root": {root} }}"#
        ))
        .expect("deserialize")
    }

    #[test]
    fn doc_defaults_fill_in() {
        let doc: SoundDoc =
            serde_json::from_str(r#"{ "name": "beep", "root": { "type": "sine", "freq": 440 } }"#)
                .unwrap();
        assert_eq!(doc.duration, 0.3);
        assert_eq!(doc.sample_rate, 44_100);
        // Version-less documents keep the pre-versioning (v1) render semantics.
        assert_eq!(doc.version, None);
        assert_eq!(doc.effective_version(), 1);
        assert!(matches!(doc.stereo, Stereo::Mono));
        assert!(matches!(doc.playback, Playback::OneShot));
    }

    #[test]
    fn v2_tracks_reject_doc_level_stereo() {
        let mut doc: SoundDoc = serde_json::from_str(
            r#"{ "name": "band", "duration": 0.2, "version": 2,
                "stereo": { "mode": "wide" },
                "root": { "type": "tracks",
                  "tracks": [ { "id": "a", "node": { "type": "sine", "freq": 220 } } ] } }"#,
        )
        .unwrap();
        let err = doc.validate().unwrap_err();
        assert!(err.contains("per-layer pan"), "{err}");
        // v1 documents keep the historical silent-ignore so old libraries load.
        doc.version = None;
        assert_eq!(doc.validate(), Ok(()));
    }

    #[test]
    fn future_schema_versions_are_rejected() {
        let mut doc: SoundDoc =
            serde_json::from_str(r#"{ "name": "beep", "root": { "type": "sine", "freq": 440 } }"#)
                .unwrap();
        assert_eq!(doc.validate(), Ok(()));
        doc.version = Some(SCHEMA_VERSION);
        assert_eq!(doc.validate(), Ok(()));
        doc.version = Some(SCHEMA_VERSION + 1);
        let err = doc.validate().unwrap_err();
        assert!(err.contains("upgrade tono"), "unhelpful error: {err}");
        doc.version = Some(0);
        assert!(doc.validate().is_err());
    }

    #[test]
    fn engine_defaults_to_zero_and_bounds_at_current_revision() {
        let mut doc: SoundDoc =
            serde_json::from_str(r#"{ "name": "beep", "root": { "type": "sine", "freq": 440 } }"#)
                .unwrap();
        // Omitted ⇒ engine 0 (the original kernels; existing docs stay bit-exact).
        assert_eq!(doc.engine, None);
        assert_eq!(doc.effective_engine(), 0);
        assert_eq!(doc.validate(), Ok(()));
        doc.engine = Some(ENGINE_VERSION);
        assert_eq!(doc.validate(), Ok(()));
        // A document from a newer DSP kernel is rejected, not misrendered.
        doc.engine = Some(ENGINE_VERSION + 1);
        let err = doc.validate().unwrap_err();
        assert!(err.contains("engine must be in"), "unhelpful error: {err}");
    }

    #[test]
    fn modal_and_impact_validate_their_ranges() {
        let modal = |modes: &str| -> Result<(), String> {
            doc_with_root(&format!(
                r#"{{ "type": "chain", "stages": [
                    {{ "type": "impact" }},
                    {{ "type": "modal", "modes": {modes} }} ] }}"#
            ))
            .validate()
        };
        assert!(modal(r#"[ { "freq": 440, "decay": 0.5, "gain": 1.0 } ]"#).is_ok());
        assert!(modal("[]").unwrap_err().contains("non-empty"));
        assert!(modal(r#"[ { "freq": -1 } ]"#).unwrap_err().contains("freq"));
        assert!(
            modal(r#"[ { "freq": 440, "decay": 0 } ]"#)
                .unwrap_err()
                .contains("decay")
        );
        assert!(
            modal(r#"[ { "freq": 440, "gain": 2 } ]"#)
                .unwrap_err()
                .contains("gain")
        );
        // Impact ranges.
        assert!(
            doc_with_root(r#"{ "type": "impact", "hardness": 1.5 }"#)
                .validate()
                .unwrap_err()
                .contains("hardness")
        );
    }

    #[test]
    fn dust_and_rand_validate_their_ranges() {
        // dust: density must be positive, decay non-negative.
        assert!(
            doc_with_root(r#"{ "type": "dust", "density": 50 }"#)
                .validate()
                .is_ok()
        );
        assert!(
            doc_with_root(r#"{ "type": "dust", "density": 0 }"#)
                .validate()
                .unwrap_err()
                .contains("density")
        );
        // rand modulator: rate must be positive.
        let with_cutoff = |m: &str| {
            doc_with_root(&format!(
                r#"{{ "type": "chain", "stages": [
                    {{ "type": "noise" }},
                    {{ "type": "lowpass", "cutoff": {m} }} ] }}"#
            ))
            .validate()
        };
        assert!(with_cutoff(r#"{ "rand": { "from": 200, "to": 1200, "rate": 0.8 } }"#).is_ok());
        assert!(
            with_cutoff(r#"{ "rand": { "from": 200, "to": 1200, "rate": 0 } }"#)
                .unwrap_err()
                .contains("rand.rate")
        );
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
    fn validate_rejects_silent_all_zero_env() {
        // The flatten footgun: nesting a/d/s/r under an "adsr" object silently
        // drops them all to 0, so the env renders pure silence.
        let d = doc(r#"{ "name": "n", "root": { "type": "env",
                "adsr": { "a": 0.01, "d": 0.1, "s": 0.7, "r": 0.2 } } }"#);
        assert!(d.validate().unwrap_err().contains("env is silent"));
        // Correctly inlined, it validates.
        let ok = doc(
            r#"{ "name": "n", "root": { "type": "env", "a": 0.01, "d": 0.1, "s": 0.7, "r": 0.2 } }"#,
        );
        assert!(ok.validate().is_ok());
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
