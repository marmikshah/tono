//! Seq synthesis: the step-sequencer voice — per-note synthesis for every
//! `SeqWave`, groove (swing / humanize), and the SoundFont sampler path.

use super::kit::{cowbell_sample, kit_drum};
use super::{Signal, adsr, eval_value, osc, poly_blep};
use crate::dsl::{
    Adsr, BassKnobs, FmKnobs, KitStyle, Node, PianoKnobs, PluckKnobs, SeqNote, SeqWave, Shape,
    TempoPoint, Value,
};
use crate::dsp::{self, Rng};
use std::f32::consts::TAU;

/// The per-seq instrument settings shared by every note.
pub(super) struct SeqVoice<'a> {
    pub(super) wave: SeqWave,
    pub(super) duty: &'a Value,
    pub(super) fm: &'a FmKnobs,
    pub(super) pluck: &'a PluckKnobs,
    pub(super) piano: &'a PianoKnobs,
    // Drum-kit voicing (the `kit` wave).
    pub(super) kit: KitStyle,
    pub(super) bass: &'a BassKnobs,
    // Read only by the SoundFont sampler path (feature = "sampler").
    #[cfg_attr(not(feature = "sampler"), allow(dead_code))]
    pub(super) sf2: &'a str,
    #[cfg_attr(not(feature = "sampler"), allow(dead_code))]
    pub(super) sf2_preset: u32,
    #[cfg_attr(not(feature = "sampler"), allow(dead_code))]
    pub(super) sf2_bank: u32,
    pub(super) swing: f32,
    pub(super) humanize: f32,
    pub(super) env: &'a Adsr,
    /// The seq's tempo map (empty = constant tempo, the f32 timing path).
    pub(super) tempo_map: &'a [TempoPoint],
}

impl<'a> SeqVoice<'a> {
    /// Borrow a `Node::Seq`'s instrument settings as a voice, along with its
    /// timing (`bpm`, `steps_per_beat`) and notes. The single construction
    /// site for every seq render path — `None` for a non-Seq node.
    pub(super) fn from_node(node: &'a Node) -> Option<(SeqVoice<'a>, f32, u32, &'a [SeqNote])> {
        let Node::Seq {
            bpm,
            tempo_map,
            steps_per_beat,
            wave,
            duty,
            fm,
            pluck,
            piano,
            kit,
            bass,
            sf2,
            swing,
            humanize,
            env,
            notes,
        } = node
        else {
            return None;
        };
        let voice = SeqVoice {
            wave: *wave,
            duty,
            fm,
            pluck,
            piano,
            kit: *kit,
            bass,
            sf2: &sf2.sf2,
            sf2_preset: sf2.sf2_preset,
            sf2_bank: sf2.sf2_bank,
            swing: *swing,
            humanize: *humanize,
            env,
            tempo_map,
        };
        Some((voice, *bpm, *steps_per_beat, notes))
    }
}

/// A stable identity for a note's pitch, mixed into the humanize
/// seed so chord notes (same step, same length) jitter independently.
fn pitch_identity(v: &Value) -> u64 {
    match v {
        Value::Const(c) => c.to_bits() as u64,
        Value::Note(s) => crate::dsp::layer_stream_key(s),
        // A per-note modulated pitch (a slide) is already unique enough by
        // its step/len in practice; all share one tag.
        Value::Modulated(_) => 0x4D4F_4455,
    }
}

/// Swing delays an off-beat by this fraction of a step at `swing: 1.0` —
/// half a step is the classic triplet-feel ceiling.
const SWING_MAX_STEP_FRACTION: f32 = 0.5;
/// Humanize timing jitter at `humanize: 1.0`, as a fraction of a step.
const HUMANIZE_TIMING_STEP_FRACTION: f32 = 0.12;
/// Humanize velocity wobble at `humanize: 1.0`, as a fraction of the gain.
const HUMANIZE_VELOCITY_FRACTION: f32 = 0.15;
/// Seed salt for the per-note humanize RNG: frac(√2)·2⁳² — an arbitrary
/// odd-looking constant that decorrelates it from the doc-seed streams.
/// PINNED: the humanize golden hashes depend on it.
const HUMANIZE_SEED_SALT: u64 = 0x6A09_E667;

/// The two per-note humanize draws (timing and velocity), seeded from the
/// note's identity so the jitter is stable per note. Callers scale them by
/// their own step duration — the constant-tempo path in f32, the tempo-map
/// path in f64 — while sharing these exact draws.
fn humanize_draws(note: &SeqNote) -> (f32, f32) {
    let mut seed = (note.step as u64) << 32 ^ (note.len as u64) << 8 ^ HUMANIZE_SEED_SALT;
    {
        seed ^= pitch_identity(&note.pitch).rotate_left(17);
    }
    let mut hr = Rng::new(seed);
    (hr.bi(), hr.bi())
}

/// Groove placement for one note: its start sample (swing + humanize timing)
/// and its humanized gain.
fn groove_note(note: &SeqNote, voice: &SeqVoice, step_dur: f32) -> (usize, f32) {
    // Swing delays every off-beat (odd) step by a fraction of a step;
    // humanize adds a deterministic per-note timing push/pull and velocity
    // wobble so repeats stop sounding machine-perfect.
    let swing_delay = if note.step % 2 == 1 {
        voice.swing * SWING_MAX_STEP_FRACTION * step_dur
    } else {
        0.0
    };
    let (human_delay, gain) = if voice.humanize > 0.0 {
        let (timing_draw, gain_draw) = humanize_draws(note);
        (
            voice.humanize * HUMANIZE_TIMING_STEP_FRACTION * step_dur * timing_draw,
            note.gain * (1.0 + voice.humanize * HUMANIZE_VELOCITY_FRACTION * gain_draw),
        )
    } else {
        (0.0, note.gain)
    };
    let start = (note.step as f32 * step_dur + swing_delay + human_delay).max(0.0) as usize;
    (start, gain.clamp(0.0, 1.0))
}

/// Segment-wise tempo-map timing: beats convert to seconds in
/// f64, segment by segment, and cross to frames once with halves rounded
/// away from zero. Notes spanning a tempo change keep their musical length.
/// The walk itself is shared ([`crate::dsl::tempo_map_seconds_at`]) so the
/// renderer and the compiler can never disagree.
struct TempoMap<'a> {
    points: &'a [TempoPoint],
}

impl TempoMap<'_> {
    /// Seconds elapsed at `beat` — the segment walk.
    fn seconds_at(&self, beat: f64) -> f64 {
        crate::dsl::tempo_map_seconds_at(self.points, beat)
    }

    /// The tempo in effect at `beat` (the local step duration derives from it).
    fn bpm_at(&self, beat: f64) -> f64 {
        crate::dsl::tempo_map_bpm_at(self.points, beat)
    }
}

/// Where a note stops, in steps (at least one step).
fn note_end_step(n: &SeqNote) -> u32 {
    n.step.saturating_add(n.len.max(1))
}

/// Groove placement under a tempo map: the same swing/humanize math as
/// [`groove_note`], scaled by the LOCAL step duration at the note's start,
/// with the final frame rounded halves-away-from-zero (the scheduling rule).
fn groove_note_mapped(
    note: &SeqNote,
    voice: &SeqVoice,
    map: &TempoMap,
    steps_per_beat: u32,
    sr: u32,
) -> (usize, usize, f32) {
    let spb = steps_per_beat as f64;
    let start_beat = note.step as f64 / spb;
    let local_step_secs = 60.0 / map.bpm_at(start_beat) / spb;
    let local_step_frames = local_step_secs * sr as f64;
    let swing_delay = if note.step % 2 == 1 {
        voice.swing as f64 * SWING_MAX_STEP_FRACTION as f64 * local_step_frames
    } else {
        0.0
    };
    let (human_delay, gain) = if voice.humanize > 0.0 {
        let (timing_draw, gain_draw) = humanize_draws(note);
        (
            voice.humanize as f64
                * HUMANIZE_TIMING_STEP_FRACTION as f64
                * local_step_frames
                * timing_draw as f64,
            note.gain * (1.0 + voice.humanize * HUMANIZE_VELOCITY_FRACTION * gain_draw),
        )
    } else {
        (0.0, note.gain)
    };
    let start_frames = map.seconds_at(start_beat) * sr as f64 + swing_delay + human_delay;
    let start = start_frames.round().max(0.0) as usize;
    let end_beat = note_end_step(note) as f64 / spb;
    let end = (map.seconds_at(end_beat) * sr as f64).round().max(0.0) as usize;
    (start, end, gain.clamp(0.0, 1.0))
}

/// Render a note sequence: each note is an instrument voice with its own
/// pitch, length, and the shared per-note ADSR, summed into the output
/// (polyphonic).
fn render_seq(
    bpm: f32,
    steps_per_beat: u32,
    voice: &SeqVoice,
    notes: &[SeqNote],
    n: usize,
    sr: u32,
    rng: &mut Rng,
) -> Signal {
    let srf = sr as f32;
    let step_dur = srf * 60.0 / bpm / steps_per_beat.max(1) as f32; // samples per step
    // The sampler plays all notes through one shared synthesizer (voices
    // interact via polyphony), so it renders the sequence as a whole.
    #[cfg(feature = "sampler")]
    if voice.wave == SeqWave::Sampler {
        return sampler_seq(voice, notes, step_dur, n, sr);
    }
    #[cfg(not(feature = "sampler"))]
    if voice.wave == SeqWave::Sampler {
        return vec![0.0f32; n];
    }
    if !voice.tempo_map.is_empty() {
        return render_seq_mapped(steps_per_beat, voice, notes, n, sr, rng);
    }
    let mut out = vec![0.0f32; n];
    for note in notes {
        let (start, gain) = groove_note(note, voice, step_dur);
        if start >= n {
            continue;
        }
        // Bound the note length by the render window BEFORE allocating: a huge
        // note.len (or tiny bpm) must not size buffers beyond what's audible.
        // (f32→usize saturates, so even an inf product stays capped by n.)
        let len = ((note.len as f32 * step_dur).min(n as f32) as usize).max(1);
        let avail = (n - start).min(len);
        let envb = adsr(voice.env, len, sr);
        let f = eval_value(&note.pitch, len, sr);
        let d = eval_value(voice.duty, len, sr);
        let sig = seq_note_signal(voice, note, &f[..avail], &d[..avail], sr, rng);
        for (i, s) in sig.into_iter().enumerate() {
            out[start + i] += s * envb[i] * gain;
        }
    }
    out
}

/// Render a note sequence whose tempo map retimes the grid: per-note
/// placement through [`groove_note_mapped`], otherwise the exact same
/// synthesis as [`render_seq`].
fn render_seq_mapped(
    steps_per_beat: u32,
    voice: &SeqVoice,
    notes: &[SeqNote],
    n: usize,
    sr: u32,
    rng: &mut Rng,
) -> Signal {
    let map = TempoMap {
        points: voice.tempo_map,
    };
    let mut out = vec![0.0f32; n];
    for note in notes {
        let (start, end, gain) = groove_note_mapped(note, voice, &map, steps_per_beat, sr);
        if start >= n {
            continue;
        }
        let len = end.saturating_sub(start).max(1).min(n);
        let avail = len.min(n - start);
        let envb = adsr(voice.env, len, sr);
        let f = eval_value(&note.pitch, len, sr);
        let d = eval_value(voice.duty, len, sr);
        let sig = seq_note_signal(voice, note, &f[..avail], &d[..avail], sr, rng);
        for (i, s) in sig.into_iter().enumerate() {
            out[start + i] += s * envb[i] * gain;
        }
    }
    out
}

/// Render a `Node::Seq` to a mono buffer with the given RNG — the exact seq
/// synthesis, shared by the offline renderer and the streaming renderer (which
/// pre-renders the seq with a structurally-seeded RNG) so a streamed seq is
/// byte-identical. Silence for a non-Seq node.
pub(crate) fn seq_to_signal(node: &Node, n: usize, sr: u32, rng: &mut Rng) -> Signal {
    if let Some((voice, bpm, steps_per_beat, notes)) = SeqVoice::from_node(node) {
        render_seq(bpm, steps_per_beat, &voice, notes, n, sr, rng)
    } else {
        vec![0.0; n]
    }
}

/// Render one note of a seq instrument: `f`/`d` are the per-sample pitch and
/// duty buffers (already truncated to the audible window). Each instrument
/// owns its per-note state; instruments that consume the PRNG (noise, pluck,
/// piano's thump, the kit) draw in sample order, keeping renders byte-exact.
fn seq_note_signal(
    voice: &SeqVoice,
    note: &SeqNote,
    f: &[f32],
    d: &[f32],
    sr: u32,
    rng: &mut Rng,
) -> Signal {
    let srf = sr as f32;
    let n = f.len();
    let mut out = Vec::with_capacity(n);
    match voice.wave {
        SeqWave::Square => {
            let mut phase = 0.0f32;
            for i in 0..n {
                let dt = f[i].max(0.0) / srf;
                let duty = d[i].clamp(0.01, 0.99);
                let mut v = if phase < duty { 1.0 } else { -1.0 };
                v += poly_blep(phase, dt);
                v -= poly_blep((phase - duty + 1.0).fract(), dt);
                out.push(v);
                phase += dt;
                phase -= phase.floor();
            }
        }
        SeqWave::Triangle => {
            let (mut phase, mut tri) = (0.0f32, 0.0f32);
            for &fi in f {
                let dt = fi.max(0.0) / srf;
                let mut sq = if phase < 0.5 { 1.0 } else { -1.0 };
                sq += poly_blep(phase, dt);
                sq -= poly_blep((phase + 0.5).fract(), dt);
                tri = tri * 0.9995 + 4.0 * dt * sq;
                out.push(tri);
                phase += dt;
                phase -= phase.floor();
            }
        }
        SeqWave::Sawtooth => {
            let mut phase = 0.0f32;
            for &fi in f {
                let dt = fi.max(0.0) / srf;
                out.push((2.0 * phase - 1.0) - poly_blep(phase, dt));
                phase += dt;
                phase -= phase.floor();
            }
        }
        SeqWave::Sine => {
            let mut phase = 0.0f32;
            for &fi in f {
                out.push(osc(Shape::Sine, phase));
                phase += fi.max(0.0) / srf;
                phase -= phase.floor();
            }
        }
        SeqWave::Noise => out.extend((0..n).map(|_| rng.bi())),
        SeqWave::Fm => {
            let (mut cph, mut mph) = (0.0f32, 0.0f32);
            for (i, &fi) in f.iter().enumerate() {
                let dt = fi.max(0.0) / srf;
                // Hammer strike: the modulation index (brightness) decays
                // from the attack; louder notes strike brighter.
                let t = i as f32 / srf;
                let idx = voice.fm.fm_index
                    * (0.4 + 0.6 * note.gain)
                    * dsp::exp(-t / voice.fm.fm_strike.max(1e-3));
                let m = idx * dsp::sin(TAU * mph);
                out.push(dsp::sin(TAU * cph + m));
                cph += dt;
                cph -= cph.floor();
                mph += dt * voice.fm.fm_ratio;
                mph -= mph.floor();
            }
        }
        SeqWave::Pluck => out = pluck_note(voice, f, sr, rng),
        SeqWave::Piano => out = piano_note(voice, note, f, sr, rng),
        SeqWave::Epiano => out = epiano_note(note, f, sr),
        SeqWave::Organ => out = organ_note(f, sr),
        SeqWave::Strings => out = strings_note(f, sr),
        SeqWave::Brass => out = brass_note(note, f, sr),
        SeqWave::Flute => out = flute_note(note, f, sr, rng),
        SeqWave::Mallet => out = mallet_note(note, f, sr),
        SeqWave::Bell => out = bell_note(f, sr),
        SeqWave::Bass => out = bass_note(voice, note, f, sr),
        SeqWave::Kit => out = kit_drum(f, sr, rng, voice.kit),
        // Handled wholesale in sampler_seq (shared synthesizer, polyphony).
        SeqWave::Sampler => unreachable!("sampler renders via sampler_seq"),
        SeqWave::Cowbell => {
            for (i, &fi) in f.iter().enumerate() {
                let t = i as f32 / srf;
                out.push(cowbell_sample(fi.max(20.0), t));
            }
        }
    }
    out
}

/// Karplus-Strong pluck: a noise burst in a delay line tuned to the note's
/// onset pitch. Three RNG-free stages wrap the loop — a tunable
/// damping/brightness filter (`pluck_tone`), a fixed guitar-body mode
/// bank (`pluck_body`), and a pick-attack click (`pluck_pick`) — each
/// an identity op at its default, so the plain pluck is byte-identical
/// and the `period` noise draws are unchanged.
fn pluck_note(voice: &SeqVoice, f: &[f32], sr: u32, rng: &mut Rng) -> Signal {
    let srf = sr as f32;
    let n = f.len();
    let mut out = Vec::with_capacity(n);
    // The .max(20.0) guard keeps the clamp ordered at absurd sample rates
    // (sr < 40), so an unvalidated doc can't panic the kernel.
    let period = ((srf / f[0].clamp(20.0, (srf / 2.0).max(20.0))).round() as usize).max(2);
    let mut string: Vec<f32> = (0..period).map(|_| rng.bi()).collect();
    let mut spos = 0usize;
    let bright = voice.pluck.pluck_tone.max(0.0);
    let damp = (-voice.pluck.pluck_tone).max(0.0);
    // Fixed guitar-body resonators: Helmholtz air, top plate, back.
    let body_r = dsp::exp(crate::dsp::NEG_LN_1000 / (0.25 * srf));
    let body_a2 = -body_r * body_r;
    let body: [(f32, f32); 3] = [(100.0, 1.0), (215.0, 0.8), (400.0, 0.5)].map(|(fr, g)| {
        let w0 = TAU * fr / srf;
        (2.0 * body_r * dsp::cos(w0), g * dsp::sin(w0)) // (a1, b0)
    });
    let (mut by1, mut by2) = ([0.0f32; 3], [0.0f32; 3]);
    let (mut lp, mut hp_in, mut hp_out) = (0.0f32, 0.0f32, 0.0f32);
    for i in 0..n {
        let t = i as f32 / srf;
        let y = string[spos];
        let next = string[(spos + 1) % string.len()];
        // Pick click: the highpassed leading edge of the excitation.
        let pick = if t < 0.008 {
            let hp = 0.9 * (hp_out + y - hp_in);
            hp_in = y;
            hp_out = hp;
            voice.pluck.pluck_pick * hp * (1.0 - t / 0.008)
        } else {
            0.0
        };
        // Body resonance driven by the string output.
        let mut body_sum = 0.0f32;
        for k in 0..3 {
            let (a1, b0) = body[k];
            let yr = b0 * y + a1 * by1[k] + body_a2 * by2[k];
            by2[k] = by1[k];
            by1[k] = yr;
            body_sum += yr;
        }
        let out_sample = (1.0 - 0.3 * voice.pluck.pluck_body) * y
            + voice.pluck.pluck_body * 0.6 * body_sum
            + pick;
        out.push(out_sample);
        // Loop filter: brightness blend then a darkening one-pole.
        let avg = (0.5 + 0.5 * bright) * y + (0.5 - 0.5 * bright) * next;
        lp += damp * (avg - lp);
        let filt = (1.0 - damp) * avg + damp * lp;
        string[spos] = voice.pluck.pluck_decay * filt;
        spos = (spos + 1) % string.len();
    }
    out
}

/// Inharmonic additive grand . A real piano string is stiff,
/// so its partials stretch sharp: fₖ = k·f₀·√(1 + B·k²). Each partial
/// owns its decay (highs die first — the bright attack mellowing to a
/// warm sustain), a hammer-strike spectrum (a 1/k tilt with a notch at
/// the ~1/8 strike point, opened by velocity), over a detuned unison
/// pair whose slow beating is the shimmer. Bass rings for seconds,
/// treble dies fast.
fn piano_note(voice: &SeqVoice, note: &SeqNote, f: &[f32], sr: u32, rng: &mut Rng) -> Signal {
    let srf = sr as f32;
    let n = f.len();
    let mut out = Vec::with_capacity(n);
    struct Partial {
        step: f32, // inharmonic frequency ratio to the fundamental
        amp: f32,  // hammer-spectrum amplitude
        env: f32,  // current decay level
        dmul: f32, // per-sample decay multiplier
        phase: [f32; 2],
    }
    // Five tone knobs (defaults reproduce the concert grand bit-for-bit).
    // piano_inharm is validated positive; the .max(0.0) keeps an unvalidated
    // negative/NaN from inverting the clamp bounds and panicking the kernel.
    let inharm = voice.piano.piano_inharm.max(0.0);
    let f0 = f[0].max(20.0);
    let b_inharm = (7.0e-5 * inharm * (f0 / 55.0)).clamp(5.0e-5 * inharm, 1.2e-3 * inharm);
    let base_decay = (10.0 * voice.piano.piano_decay / (1.0 + f0 / 110.0)).clamp(0.45, 9.0);
    let strike = voice.piano.piano_strike.clamp(0.01, 0.5); // hammer position along the string
    let bright = 0.45 + 0.55 * note.gain; // velocity opens the high partials
    let hammer = voice.piano.piano_hammer.max(1e-3); // hardness: flattens the tilt
    let detune = 1.0 + (1.000_6_f32 - 1.0) * voice.piano.piano_detune; // unison spread
    let string_det = [1.0 / detune, detune];

    let mut partials: Vec<Partial> = Vec::new();
    let mut k = 1usize;
    while k <= 18 {
        let kf = k as f32;
        let ratio = kf * (1.0 + b_inharm * kf * kf).sqrt();
        if ratio * f0 > 0.45 * srf {
            break; // keep every partial below Nyquist
        }
        let notch = dsp::sin(std::f32::consts::PI * kf * strike).abs();
        let amp = notch / kf * dsp::powf(bright, (kf - 1.0) * 0.18 / hammer);
        let decay = (base_decay / (1.0 + 0.55 * (kf - 1.0))).max(0.05);
        partials.push(Partial {
            step: ratio,
            amp,
            env: 1.0,
            dmul: dsp::exp(-1.0 / (srf * decay)),
            // Spread start phases (golden ratio) so the onset isn't a
            // hard in-phase transient — deterministic, no RNG draw.
            phase: [(kf * 0.618_034).fract(), (kf * 0.381_966).fract()],
        });
        k += 1;
    }
    // Target a per-note peak near 0.5 (as the FM model had): two strings
    // over the summed partial amplitude.
    let norm = 0.5 / (2.0 * partials.iter().map(|p| p.amp).sum::<f32>().max(1e-6));

    for (i, &fi) in f.iter().enumerate() {
        let dt = fi.max(0.0) / srf;
        let t = i as f32 / srf;
        let mut s = 0.0;
        for p in partials.iter_mut() {
            let inc = dt * p.step;
            let a = p.amp * p.env;
            for (ph, &det) in p.phase.iter_mut().zip(string_det.iter()) {
                s += a * dsp::sin(TAU * *ph);
                *ph += inc * det;
                *ph -= ph.floor();
            }
            p.env *= p.dmul;
        }
        // Felt-hammer thump: a few ms of soft noise on the attack.
        let thump = if t < 0.006 {
            rng.bi() * 0.3 * (1.0 - t / 0.006)
        } else {
            0.0
        };
        out.push(s * norm + thump);
    }
    out
}

/// Rhodes-style e-piano: a soft FM body (1:1) under a metal tine (14:1)
/// that pings on the attack. Velocity opens the tine.
fn epiano_note(note: &SeqNote, f: &[f32], sr: u32) -> Signal {
    let srf = sr as f32;
    let n = f.len();
    let mut out = Vec::with_capacity(n);
    let decay = (5.0 / (1.0 + f[0].max(20.0) / 250.0)).clamp(0.3, 4.0);
    let (mut cph, mut mph, mut tph) = (0.0f32, 0.0f32, 0.0f32);
    for (i, &fi) in f.iter().enumerate() {
        let dt = fi.max(0.0) / srf;
        let t = i as f32 / srf;
        let body_idx = (0.5 + 1.0 * note.gain) * dsp::exp(-t / 0.5);
        let tine_idx = (0.8 + 1.4 * note.gain) * dsp::exp(-t / 0.035);
        let body = dsp::sin(TAU * cph + body_idx * dsp::sin(TAU * mph));
        let tine = dsp::sin(TAU * cph + tine_idx * dsp::sin(TAU * tph));
        cph += dt;
        cph -= cph.floor();
        mph += dt;
        mph -= mph.floor();
        tph += dt * 14.0;
        tph -= tph.floor();
        out.push((0.75 * body + 0.25 * tine) * dsp::exp(-t / decay));
    }
    out
}

/// Tonewheel organ: drawbars over half the fundamental (so the 16′ bar is
/// an integer partial and every phase wraps cleanly): 16′ 8′ 4′ 2⅔′ 2′,
/// plus the classic percussion ping on the attack.
fn organ_note(f: &[f32], sr: u32) -> Signal {
    let srf = sr as f32;
    let n = f.len();
    let mut out = Vec::with_capacity(n);
    const BARS: [(f32, f32); 5] = [
        (1.0, 0.45),
        (2.0, 1.0),
        (4.0, 0.45),
        (6.0, 0.3),
        (8.0, 0.22),
    ];
    let norm = 1.0 / BARS.iter().map(|(_, g)| g).sum::<f32>();
    let mut phase = 0.0f32; // at f/2
    for (i, &fi) in f.iter().enumerate() {
        let t = i as f32 / srf;
        let mut s = 0.0;
        for (k, g) in BARS {
            s += g * dsp::sin(TAU * phase * k);
        }
        // Percussion: a 3rd-harmonic ping that fades in 200 ms.
        s += 0.5 * dsp::exp(-t / 0.2) * dsp::sin(TAU * phase * 6.0);
        out.push(s * norm);
        phase += fi.max(0.0) / 2.0 / srf;
        // Wrap on the full drawbar cycle to keep precision.
        phase -= phase.floor();
    }
    out
}

/// String ensemble: three saws detuned ±8 cents, phase-spread, swelling
/// in like a bow stroke, mellowed by a one-pole lowpass.
fn strings_note(f: &[f32], sr: u32) -> Signal {
    let srf = sr as f32;
    let n = f.len();
    let mut out = Vec::with_capacity(n);
    let detunes = [0.995_39f32, 1.0, 1.004_63]; // ∓8 cents
    let mut phases = [0.0f32, 0.33, 0.67];
    let lp_a = 1.0 - dsp::exp(-TAU * 3_000.0 / srf);
    let mut lp = 0.0f32;
    for (i, &fi) in f.iter().enumerate() {
        let t = i as f32 / srf;
        let mut s = 0.0;
        for (p, det) in phases.iter_mut().zip(detunes) {
            let dt = fi.max(0.0) * det / srf;
            s += (2.0 * *p - 1.0) - poly_blep(*p, dt);
            *p += dt;
            *p -= p.floor();
        }
        lp += lp_a * (s / 3.0 - lp);
        let swell = 1.0 - dsp::exp(-t / 0.12);
        out.push(lp * swell);
    }
    out
}

/// Brass: two band-limited saws detuned ±0.4% through a one-pole lowpass
/// whose cutoff swells from ~800 Hz toward ~2.2 kHz over the first ~70 ms —
/// the lip-reed "blat" of a horn attack. Velocity opens the filter further.
fn brass_note(note: &SeqNote, f: &[f32], sr: u32) -> Signal {
    let srf = sr as f32;
    let n = f.len();
    let mut out = Vec::with_capacity(n);
    let detunes = [0.996f32, 1.004];
    let mut phases = [0.0f32, 0.5]; // phase-spread so the onset isn't a click
    let mut lp = 0.0f32;
    for (i, &fi) in f.iter().enumerate() {
        let t = i as f32 / srf;
        let mut s = 0.0;
        for (p, det) in phases.iter_mut().zip(detunes) {
            let dt = fi.max(0.0) * det / srf;
            s += (2.0 * *p - 1.0) - poly_blep(*p, dt);
            *p += dt;
            *p -= p.floor();
        }
        // The blat: cutoff opens over ~70 ms; velocity pushes it brighter.
        let cutoff = 800.0 + (1_400.0 + 1_200.0 * note.gain) * (1.0 - dsp::exp(-t / 0.07));
        let a = 1.0 - dsp::exp(-TAU * cutoff.min(0.45 * srf) / srf);
        lp += a * (s * 0.5 - lp);
        out.push(lp);
    }
    out
}

/// Concert flute: a sine with a ~5.5 Hz vibrato that fades in over ~150 ms
/// (a player settling into the note), over breath — white noise through a
/// gentle one-pole lowpass, louder with velocity. One RNG draw per sample,
/// in sample order, so chunking can't change the output.
fn flute_note(note: &SeqNote, f: &[f32], sr: u32, rng: &mut Rng) -> Signal {
    let srf = sr as f32;
    let n = f.len();
    let mut out = Vec::with_capacity(n);
    let lp_a = 1.0 - dsp::exp(-TAU * 1_200.0 / srf);
    let (mut phase, mut vib, mut lp) = (0.0f32, 0.0f32, 0.0f32);
    for (i, &fi) in f.iter().enumerate() {
        let t = i as f32 / srf;
        // Vibrato swells in after the attack, to ±0.6% of pitch.
        let depth = 0.006 * (1.0 - dsp::exp(-t / 0.15));
        let fmod = 1.0 + depth * dsp::sin(TAU * vib);
        vib += 5.5 / srf;
        vib -= vib.floor();
        phase += fi.max(0.0) * fmod / srf;
        phase -= phase.floor();
        // Breath: one draw per sample, lowpassed to an airy hiss.
        lp += lp_a * (rng.bi() - lp);
        out.push(0.9 * dsp::sin(TAU * phase) + (0.02 + 0.03 * note.gain) * lp);
    }
    out
}

/// Marimba-like mallet: a sine fundamental with two fast-decaying strike
/// partials (ratios ~3.9 and ~9.2 — the wooden bar modes) that die in tens
/// of milliseconds. Velocity brightens the strike. Woodier and
/// shorter-lived than the e-piano.
fn mallet_note(note: &SeqNote, f: &[f32], sr: u32) -> Signal {
    let srf = sr as f32;
    let n = f.len();
    let mut out = Vec::with_capacity(n);
    let bright = 0.4 + 0.6 * note.gain;
    let mut ph = [0.0f32; 3];
    for (i, &fi) in f.iter().enumerate() {
        let dt = fi.max(0.0) / srf;
        let t = i as f32 / srf;
        let s = dsp::sin(TAU * ph[0])
            + bright * 0.5 * dsp::exp(-t / 0.03) * dsp::sin(TAU * ph[1])
            + bright * 0.25 * dsp::exp(-t / 0.01) * dsp::sin(TAU * ph[2]);
        out.push(0.7 * s);
        ph[0] += dt;
        ph[1] += dt * 3.9;
        ph[2] += dt * 9.2;
        for p in ph.iter_mut() {
            *p -= p.floor();
        }
    }
    out
}

/// Struck bell: additive inharmonic partials (1, 2.02, 2.74, 4.07, 5.43 —
/// the hum/prime/tierce stack of a real bell), each with its own decay so
/// the highs die first and the hum rings on, plus a 0.1% detuned twin of
/// the fundamental whose slow beating is the shimmer.
fn bell_note(f: &[f32], sr: u32) -> Signal {
    let srf = sr as f32;
    let n = f.len();
    let mut out = Vec::with_capacity(n);
    // (ratio, gain, decay secs).
    const PARTIALS: [(f32, f32, f32); 5] = [
        (1.0, 1.0, 3.0),
        (2.02, 0.6, 1.5),
        (2.74, 0.4, 0.8),
        (4.07, 0.3, 0.4),
        (5.43, 0.2, 0.25),
    ];
    let mut phases = [0.0f32; 6]; // 5 partials + the detuned twin
    let norm = 1.0 / (1.0 + PARTIALS.iter().map(|(_, g, _)| g).sum::<f32>());
    for (i, &fi) in f.iter().enumerate() {
        let dt = fi.max(0.0) / srf;
        let t = i as f32 / srf;
        let mut s = 0.0;
        for (j, &(ratio, g, decay)) in PARTIALS.iter().enumerate() {
            s += g * dsp::exp(-t / decay.max(1e-3)) * dsp::sin(TAU * phases[j]);
            phases[j] += dt * ratio;
            phases[j] -= phases[j].floor();
        }
        // 0.1% detuned twin of the fundamental: the slow-beat shimmer.
        s += dsp::exp(-t / 3.0) * dsp::sin(TAU * phases[5]);
        phases[5] += dt * 1.001;
        phases[5] -= phases[5].floor();
        out.push(s * norm);
    }
    out
}

/// Fingered/synth bass: a saw through a velocity-swept one-pole lowpass over
/// a sine sub. Every constant is a `bass_*` knob; the defaults reproduce the
/// original fingered bass bit-for-bit (and draw no RNG, so it streams
/// byte-identically). `bass_click` adds a deterministic pick tick,
/// `bass_drive` a tanh grit, `bass_sub_ratio` an octave-down sub.
fn bass_note(voice: &SeqVoice, note: &SeqNote, f: &[f32], sr: u32) -> Signal {
    let srf = sr as f32;
    let n = f.len();
    let mut out = Vec::with_capacity(n);
    const BASS_CLICK_TAU: f32 = 0.008;
    let decay = voice.bass.bass_decay.max(1e-3);
    let body_decay = voice.bass.bass_body_decay.max(1e-3);
    let drive = voice.bass.bass_drive.clamp(0.0, 1.0);
    let mut phase = 0.0f32;
    let mut sub_phase = 0.0f32;
    let mut lp = 0.0f32;
    for (i, &fi) in f.iter().enumerate() {
        let dt = fi.max(0.0) / srf;
        let t = i as f32 / srf;
        let saw = (2.0 * phase - 1.0) - poly_blep(phase, dt);
        let cutoff = voice.bass.bass_cutoff
            + (voice.bass.bass_env + voice.bass.bass_env_vel * note.gain) * dsp::exp(-t / decay)
            + voice.bass.bass_click * dsp::exp(-t / BASS_CLICK_TAU);
        let a = 1.0 - dsp::exp(-TAU * cutoff / srf);
        lp += a * (saw - lp);
        let body = lp + drive * (dsp::tanh(lp * (1.0 + 2.0 * drive)) - lp);
        let sub = dsp::sin(TAU * sub_phase);
        out.push(
            (voice.bass.bass_body * body + voice.bass.bass_sub * sub) * dsp::exp(-t / body_decay),
        );
        phase += dt;
        phase -= phase.floor();
        sub_phase += dt * voice.bass.bass_sub_ratio;
        sub_phase -= sub_phase.floor();
    }
    out
}

/// Render a whole sampler seq through rustysynth: real recorded instruments
/// from a SoundFont. All notes share one synthesizer so polyphony, voice
/// stealing, and per-preset envelopes behave like a real MIDI instrument.
/// Output is the stereo render downmixed to the graph's mono bus (doc-level
/// `stereo` adds width back at the output stage).
#[cfg(feature = "sampler")]
fn sampler_seq(voice: &SeqVoice, notes: &[SeqNote], step_dur: f32, n: usize, sr: u32) -> Signal {
    match sampler_seq_stereo(voice, notes, step_dur, n, sr) {
        Some((l, r)) => l.iter().zip(r).map(|(a, b)| 0.5 * (a + b)).collect(),
        None => vec![0.0; n],
    }
}

/// The sampler's native stereo render (used directly by mixer tracks).
#[cfg(feature = "sampler")]
pub(super) fn sampler_seq_stereo(
    voice: &SeqVoice,
    notes: &[SeqNote],
    step_dur: f32,
    n: usize,
    sr: u32,
) -> Option<(Signal, Signal)> {
    use rustysynth::{Synthesizer, SynthesizerSettings};

    let font = match load_soundfont(voice.sf2) {
        Ok(f) => f,
        Err(e) => {
            tracing::warn!("sampler: cannot load '{}': {e}", voice.sf2);
            return None;
        }
    };
    let mut settings = SynthesizerSettings::new(sr as i32);
    // Our graph supplies reverb/chorus as explicit processors; the synth's
    // built-ins stay off so renders are lean and deterministic.
    settings.enable_reverb_and_chorus = false;
    let mut synth = match Synthesizer::new(&font, &settings) {
        Ok(s) => s,
        Err(e) => {
            tracing::warn!("sampler: synthesizer init failed: {e:?}");
            return None;
        }
    };
    // Channel 9 is percussion by MIDI convention; bank 128 selects it.
    let ch = if voice.sf2_bank == 128 { 9 } else { 0 };
    synth.process_midi_message(ch, 0xC0, voice.sf2_preset.min(127) as i32, 0);

    // Schedule note on/offs on the sample timeline (groove applied).
    let mut events: Vec<(usize, bool, i32, i32)> = Vec::with_capacity(notes.len() * 2);
    for note in notes {
        let (start, gain) = groove_note(note, voice, step_dur);
        if start >= n {
            continue;
        }
        let len = ((note.len as f32 * step_dur).min(n as f32) as usize).max(1);
        let hz = eval_value(&note.pitch, 1, sr)[0].max(8.0);
        let key = crate::dsp::hz_to_midi(hz).round() as i32;
        let vel = ((gain * 127.0) as i32).clamp(1, 127);
        events.push((start, true, key.clamp(0, 127), vel));
        events.push(((start + len).min(n), false, key.clamp(0, 127), 0));
    }
    // Offs before ons at the same instant, so retriggers restart the voice.
    events.sort_by_key(|&(at, is_on, ..)| (at, is_on));

    let (mut left, mut right) = (vec![0.0f32; n], vec![0.0f32; n]);
    let mut pos = 0usize;
    for (at, is_on, key, vel) in events {
        if at > pos {
            let (lh, rh) = (&mut left[pos..at], &mut right[pos..at]);
            synth.render(lh, rh);
            pos = at;
        }
        if is_on {
            synth.note_on(ch, key, vel);
        } else {
            synth.note_off(ch, key);
        }
    }
    if pos < n {
        synth.render(&mut left[pos..], &mut right[pos..]);
    }
    Some((left, right))
}

/// SoundFonts are large; load each file once per process and share it.
///
/// This is the core's SOLE, feature-gated I/O exemption ("no I/O" otherwise):
/// multi-MB external assets are addressed by path in the DSL, so the render
/// path must open them. Loaders pre-check existence via `SoundDoc::sf2_paths`.
#[cfg(feature = "sampler")]
fn load_soundfont(path: &str) -> anyhow::Result<std::sync::Arc<rustysynth::SoundFont>> {
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex, OnceLock};
    static CACHE: OnceLock<Mutex<HashMap<String, Arc<rustysynth::SoundFont>>>> = OnceLock::new();
    let cache = CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    if let Some(f) = cache.lock().unwrap_or_else(|e| e.into_inner()).get(path) {
        return Ok(f.clone());
    }
    // Deliberately lock-free across the parse: two concurrent renders of the
    // same font may both parse (last insert wins — same bytes), which is
    // strictly better than serializing renders on a multi-MB parse.
    let mut file = std::fs::File::open(path)?;
    let font = Arc::new(
        rustysynth::SoundFont::new(&mut file).map_err(|e| anyhow::anyhow!("parse: {e:?}"))?,
    );
    cache
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(path.to_string(), font.clone());
    Ok(font)
}
