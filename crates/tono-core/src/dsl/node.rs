//! The synthesis-graph [`Node`] enum — every source, combinator, and
//! processor in the DSL — with its per-family serde defaults co-located.

use super::{
    Adsr, DriveShape, KitStyle, Mode, NoiseColor, SeqNote, SeqWave, SuperWave, Track, Value,
    default_gain,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

// Serde `default = "..."` requires free functions. Values with non-obvious
// origins: q 0.707 is Butterworth (maximally flat).
fn default_duty() -> Value {
    Value::Const(0.5)
}
fn default_q() -> f32 {
    0.707
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
// Guitar tone stages on the `pluck` voice. All default to identity (0.0), so
// omitting them renders byte-identically and draws no extra RNG.
fn default_pluck_body() -> f32 {
    0.0
}
fn default_pluck_pick() -> f32 {
    0.0
}
fn default_pluck_tone() -> f32 {
    0.0
}
// Piano tone knobs (engine-3 additive `piano` voice). Every default reproduces
// the concert-grand kernel bit-for-bit (x*1.0==x, x/1.0==x, 0.125==1.0/8.0 in
// f32), so a doc that omits them renders byte-identically. Variants set others.
fn default_piano_hammer() -> f32 {
    1.0
}
fn default_piano_strike() -> f32 {
    0.125
}
fn default_piano_inharm() -> f32 {
    1.0
}
fn default_piano_detune() -> f32 {
    1.0
}
fn default_piano_decay() -> f32 {
    1.0
}
// Bass tone knobs (the `bass` voice). Every default is the current voice's
// hard-coded constant, so omitting them renders byte-identically.
fn default_bass_cutoff() -> f32 {
    250.0
}
fn default_bass_env() -> f32 {
    700.0
}
fn default_bass_env_vel() -> f32 {
    1100.0
}
fn default_bass_decay() -> f32 {
    0.15
}
fn default_bass_click() -> f32 {
    0.0
}
fn default_bass_body() -> f32 {
    0.7
}
fn default_bass_sub() -> f32 {
    0.45
}
fn default_bass_sub_ratio() -> f32 {
    1.0
}
fn default_bass_drive() -> f32 {
    0.0
}
fn default_bass_body_decay() -> f32 {
    2.0
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
fn default_modal_mix() -> f32 {
    1.0
}
fn default_impact_hardness() -> f32 {
    0.5
}
fn default_dust_decay() -> f32 {
    0.02
}

/// A node in the synthesis graph. Every node evaluates to a mono signal.
#[non_exhaustive]
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
        /// FM voice knobs (`wave: "fm"`), flattened onto the node.
        #[serde(flatten)]
        fm: FmKnobs,
        /// Plucked-string knobs (`wave: "pluck"`), flattened onto the node.
        #[serde(flatten)]
        pluck: PluckKnobs,
        /// Piano tone knobs (`wave: "piano"`, engine ≥ 3), flattened onto the node.
        #[serde(flatten)]
        piano: PianoKnobs,
        /// Drum-kit voicing when `wave` is `kit`. Omitted ⇒ `classic` (the
        /// original kit, byte-identical); `acoustic`/`electronic`/`808` are
        /// alternate synthesized kits.
        #[serde(default)]
        kit: KitStyle,
        /// Bass tone knobs (`wave: "bass"`), flattened onto the node.
        #[serde(flatten)]
        bass: BassKnobs,
        /// SoundFont sampler settings (`wave: "sampler"`), flattened onto the node.
        #[serde(flatten)]
        sf2: Sf2Knobs,
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

/// FM voice knobs of a `seq` node (`wave: "fm"`), flattened onto the node in
/// JSON. Defaults ≈ an FM piano strike (ratio 1 + decaying index).
#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema)]
pub struct FmKnobs {
    /// Modulator frequency ratio (1 = e-piano/piano, 3.5 = bell, 14 = tine).
    #[serde(default = "default_seq_fm_ratio")]
    pub fm_ratio: f32,
    /// Modulation index at the strike (brightness; also scaled by each note's
    /// velocity, so louder notes ring brighter).
    #[serde(default = "default_seq_fm_index")]
    pub fm_index: f32,
    /// Strike decay in seconds: how fast the index (brightness) fades after
    /// each note's attack. Short = percussive e-piano, long = bell shimmer.
    #[serde(default = "default_seq_fm_strike")]
    pub fm_strike: f32,
}

/// Plucked-string knobs of a `seq` node (`wave: "pluck"`), flattened onto the
/// node in JSON. The tone stages default to identity, so omitting them renders
/// byte-identically and draws no extra RNG.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema)]
pub struct PluckKnobs {
    /// String feedback decay, 0.8..1 (higher rings longer; low notes
    /// naturally ring longer than high ones).
    #[serde(default = "default_pluck_decay")]
    pub pluck_decay: f32,
    /// Acoustic body-resonance depth, 0..1 — mixes in a fixed guitar-body
    /// mode bank. 0 = solid-body (default).
    #[serde(default = "default_pluck_body")]
    pub pluck_body: f32,
    /// Pick/attack transient level, 0..1. 0 = none.
    #[serde(default = "default_pluck_pick")]
    pub pluck_pick: f32,
    /// String brightness/damping, −1..1. 0 = the current loop filter;
    /// + brightens, − darkens.
    #[serde(default = "default_pluck_tone")]
    pub pluck_tone: f32,
}

/// Piano tone knobs of a `seq` node (`wave: "piano"`, engine ≥ 3), flattened
/// onto the node in JSON. Every default reproduces the concert-grand kernel
/// bit-for-bit, so a doc that omits them renders byte-identically.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema)]
pub struct PianoKnobs {
    /// Hammer hardness: spectral brightness. 1 = concert grand; > 1
    /// harder/brighter, < 1 softer/darker.
    #[serde(default = "default_piano_hammer")]
    pub piano_hammer: f32,
    /// Hammer strike position (fraction along the string): the comb-notch
    /// that thins the spectrum. 0.125 = grand; toward the bridge is brighter.
    #[serde(default = "default_piano_strike")]
    pub piano_strike: f32,
    /// String-stiffness scale: stretches the partials sharp. 1 = grand;
    /// > 1 = short/stiff upright jangle.
    #[serde(default = "default_piano_inharm")]
    pub piano_inharm: f32,
    /// Unison detune width: the two-string beating. 1 ≈ ±1 cent (grand
    /// shimmer); ~12 = honky-tonk warble.
    #[serde(default = "default_piano_detune")]
    pub piano_detune: f32,
    /// Ring-time scale. 1 = grand; < 1 = shorter, damped; > 1 = longer.
    #[serde(default = "default_piano_decay")]
    pub piano_decay: f32,
}

/// Bass tone knobs of a `seq` node (`wave: "bass"`), flattened onto the node
/// in JSON. Every default is the original voice's hard-coded constant, so
/// omitting them renders byte-identically.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema)]
pub struct BassKnobs {
    /// Filter resting floor in Hz. Low = dark/round.
    #[serde(default = "default_bass_cutoff")]
    pub bass_cutoff: f32,
    /// Fixed cutoff-sweep depth above the floor, Hz.
    #[serde(default = "default_bass_env")]
    pub bass_env: f32,
    /// Velocity-scaled sweep depth, Hz (× note velocity).
    #[serde(default = "default_bass_env_vel")]
    pub bass_env_vel: f32,
    /// Filter-sweep time constant, seconds — how fast the cutoff closes.
    #[serde(default = "default_bass_decay")]
    pub bass_decay: f32,
    /// Pick-tick: an extra attack cutoff bump over ~8 ms, Hz. 0 = none.
    #[serde(default = "default_bass_click")]
    pub bass_click: f32,
    /// Filtered-saw body level.
    #[serde(default = "default_bass_body")]
    pub bass_body: f32,
    /// Sine-sub level.
    #[serde(default = "default_bass_sub")]
    pub bass_sub: f32,
    /// Sub frequency ratio to the note. 1 = reinforce; 0.5 = octave down.
    #[serde(default = "default_bass_sub_ratio")]
    pub bass_sub_ratio: f32,
    /// tanh saturation, 0..1. 0 = clean; > 0 = synth-bass grit.
    #[serde(default = "default_bass_drive")]
    pub bass_drive: f32,
    /// Note body decay, seconds (on top of the ADSR). Longer = sustained.
    #[serde(default = "default_bass_body_decay")]
    pub bass_body_decay: f32,
}

/// SoundFont sampler settings of a `seq` node (`wave: "sampler"`), flattened
/// onto the node in JSON.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct Sf2Knobs {
    /// Path to a SoundFont (.sf2) file.
    #[serde(default)]
    pub sf2: String,
    /// General MIDI program number (0..=127).
    #[serde(default)]
    pub sf2_preset: u32,
    /// SoundFont bank (0 = melodic, 128 = the percussion bank / GM drum map).
    #[serde(default)]
    pub sf2_bank: u32,
}
