//! Seq synthesis: the step-sequencer voice — per-note synthesis for every
//! `SeqWave`, groove (swing / humanize), and the SoundFont sampler path.

use super::kit::{cowbell_sample, kit_drum};
use super::{Signal, adsr, eval_value, osc, poly_blep};
use crate::dsl::{Adsr, KitStyle, Node, SeqNote, SeqWave, Shape, Value};
use crate::dsp::Rng;
use std::f32::consts::TAU;

/// The per-seq instrument settings shared by every note.
pub(super) struct SeqVoice<'a> {
    pub(super) wave: SeqWave,
    pub(super) duty: &'a Value,
    pub(super) fm_ratio: f32,
    pub(super) fm_index: f32,
    pub(super) fm_strike: f32,
    pub(super) pluck_decay: f32,
    // Guitar tone stages (the `pluck` voice).
    pub(super) pluck_body: f32,
    pub(super) pluck_pick: f32,
    pub(super) pluck_tone: f32,
    // Piano tone knobs (engine-3 additive `piano` voice).
    pub(super) piano_hammer: f32,
    pub(super) piano_strike: f32,
    pub(super) piano_inharm: f32,
    pub(super) piano_detune: f32,
    pub(super) piano_decay: f32,
    // Drum-kit voicing (the `kit` wave).
    pub(super) kit: KitStyle,
    // Bass tone knobs (the `bass` wave).
    pub(super) bass_cutoff: f32,
    pub(super) bass_env: f32,
    pub(super) bass_env_vel: f32,
    pub(super) bass_decay: f32,
    pub(super) bass_click: f32,
    pub(super) bass_body: f32,
    pub(super) bass_sub: f32,
    pub(super) bass_sub_ratio: f32,
    pub(super) bass_drive: f32,
    pub(super) bass_body_decay: f32,
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
    /// The document's engine revision — gates byte-changing voice upgrades
    /// (e.g. the engine-3 inharmonic `piano`).
    pub(super) engine: u32,
}

/// A stable identity for a note's pitch, mixed into the engine ≥ 4 humanize
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

/// Groove placement for one note: its start sample (swing + humanize timing)
/// and its humanized gain.
fn groove_note(note: &SeqNote, voice: &SeqVoice, step_dur: f32) -> (usize, f32) {
    // Swing delays every off-beat (odd) step by a fraction of a step;
    // humanize adds a deterministic per-note timing push/pull and velocity
    // wobble so repeats stop sounding machine-perfect.
    let swing_delay = if note.step % 2 == 1 {
        voice.swing * 0.5 * step_dur
    } else {
        0.0
    };
    let (human_delay, gain) = if voice.humanize > 0.0 {
        // Seed from the note's identity so the jitter is stable per note.
        let mut seed = (note.step as u64) << 32 ^ (note.len as u64) << 8 ^ 0x6A09_E667;
        if voice.engine >= 4 {
            // Chord-aware: seeded from (step, len) alone, every note of a
            // chord shared one timing/velocity offset and moved as a block —
            // a human never does that. Engine ≤ 3 keeps the shared seed
            // bit-for-bit.
            seed ^= pitch_identity(&note.pitch).rotate_left(17);
        }
        let mut hr = Rng::new(seed);
        (
            voice.humanize * 0.12 * step_dur * hr.bi(),
            note.gain * (1.0 + voice.humanize * 0.15 * hr.bi()),
        )
    } else {
        (0.0, note.gain)
    };
    let start = (note.step as f32 * step_dur + swing_delay + human_delay).max(0.0) as usize;
    (start, gain.clamp(0.0, 1.0))
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

/// Render a `Node::Seq` to a mono buffer with the given RNG — the exact seq
/// synthesis, shared by the offline renderer and the streaming renderer (which
/// pre-renders the seq with a structurally-seeded RNG) so a streamed seq is
/// byte-identical. Silence for a non-Seq node.
pub(crate) fn seq_to_signal(node: &Node, n: usize, sr: u32, rng: &mut Rng, engine: u32) -> Signal {
    if let Node::Seq {
        bpm,
        steps_per_beat,
        wave,
        duty,
        fm_ratio,
        fm_index,
        fm_strike,
        pluck_decay,
        pluck_body,
        pluck_pick,
        pluck_tone,
        piano_hammer,
        piano_strike,
        piano_inharm,
        piano_detune,
        piano_decay,
        kit,
        bass_cutoff,
        bass_env,
        bass_env_vel,
        bass_decay,
        bass_click,
        bass_body,
        bass_sub,
        bass_sub_ratio,
        bass_drive,
        bass_body_decay,
        sf2,
        sf2_preset,
        sf2_bank,
        swing,
        humanize,
        env,
        notes,
    } = node
    {
        let voice = SeqVoice {
            wave: *wave,
            duty,
            fm_ratio: *fm_ratio,
            fm_index: *fm_index,
            fm_strike: *fm_strike,
            pluck_decay: *pluck_decay,
            pluck_body: *pluck_body,
            pluck_pick: *pluck_pick,
            pluck_tone: *pluck_tone,
            piano_hammer: *piano_hammer,
            piano_strike: *piano_strike,
            piano_inharm: *piano_inharm,
            piano_detune: *piano_detune,
            piano_decay: *piano_decay,
            kit: *kit,
            bass_cutoff: *bass_cutoff,
            bass_env: *bass_env,
            bass_env_vel: *bass_env_vel,
            bass_decay: *bass_decay,
            bass_click: *bass_click,
            bass_body: *bass_body,
            bass_sub: *bass_sub,
            bass_sub_ratio: *bass_sub_ratio,
            bass_drive: *bass_drive,
            bass_body_decay: *bass_body_decay,
            sf2,
            sf2_preset: *sf2_preset,
            sf2_bank: *sf2_bank,
            swing: *swing,
            humanize: *humanize,
            env,
            engine,
        };
        render_seq(*bpm, *steps_per_beat, &voice, notes, n, sr, rng)
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
                let idx = voice.fm_index
                    * (0.4 + 0.6 * note.gain)
                    * (-t / voice.fm_strike.max(1e-3)).exp();
                let m = idx * (TAU * mph).sin();
                out.push((TAU * cph + m).sin());
                cph += dt;
                cph -= cph.floor();
                mph += dt * voice.fm_ratio;
                mph -= mph.floor();
            }
        }
        SeqWave::Pluck => {
            // Karplus-Strong: a noise burst in a delay line tuned to the note's
            // onset pitch. Three RNG-free stages wrap the loop — a tunable
            // damping/brightness filter (`pluck_tone`), a fixed guitar-body mode
            // bank (`pluck_body`), and a pick-attack click (`pluck_pick`) — each
            // an identity op at its default, so the plain pluck is byte-identical
            // and the `period` noise draws are unchanged.
            let period = ((srf / f[0].clamp(20.0, srf / 2.0)).round() as usize).max(2);
            let mut string: Vec<f32> = (0..period).map(|_| rng.bi()).collect();
            let mut spos = 0usize;
            let bright = voice.pluck_tone.max(0.0);
            let damp = (-voice.pluck_tone).max(0.0);
            // Fixed guitar-body resonators: Helmholtz air, top plate, back.
            let body_r = (-6.907_755 / (0.25 * srf)).exp();
            let body_a2 = -body_r * body_r;
            let body: [(f32, f32); 3] =
                [(100.0, 1.0), (215.0, 0.8), (400.0, 0.5)].map(|(fr, g)| {
                    let w0 = TAU * fr / srf;
                    (2.0 * body_r * w0.cos(), g * w0.sin()) // (a1, b0)
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
                    voice.pluck_pick * hp * (1.0 - t / 0.008)
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
                let out_sample =
                    (1.0 - 0.3 * voice.pluck_body) * y + voice.pluck_body * 0.6 * body_sum + pick;
                out.push(out_sample);
                // Loop filter: brightness blend then a darkening one-pole.
                let avg = (0.5 + 0.5 * bright) * y + (0.5 - 0.5 * bright) * next;
                lp += damp * (avg - lp);
                let filt = (1.0 - damp) * avg + damp * lp;
                string[spos] = voice.pluck_decay * filt;
                spos = (spos + 1) % string.len();
            }
        }
        SeqWave::Piano if voice.engine >= 3 => {
            // Inharmonic additive grand (engine 3). A real piano string is stiff,
            // so its partials stretch sharp: fₖ = k·f₀·√(1 + B·k²). Each partial
            // owns its decay (highs die first — the bright attack mellowing to a
            // warm sustain), a hammer-strike spectrum (a 1/k tilt with a notch at
            // the ~1/8 strike point, opened by velocity), over a detuned unison
            // pair whose slow beating is the shimmer. Bass rings for seconds,
            // treble dies fast.
            struct Partial {
                step: f32, // inharmonic frequency ratio to the fundamental
                amp: f32,  // hammer-spectrum amplitude
                env: f32,  // current decay level
                dmul: f32, // per-sample decay multiplier
                phase: [f32; 2],
            }
            // Five tone knobs (defaults reproduce the concert grand bit-for-bit).
            let f0 = f[0].max(20.0);
            let b_inharm = (7.0e-5 * voice.piano_inharm * (f0 / 55.0))
                .clamp(5.0e-5 * voice.piano_inharm, 1.2e-3 * voice.piano_inharm);
            let base_decay = (10.0 * voice.piano_decay / (1.0 + f0 / 110.0)).clamp(0.45, 9.0);
            let strike = voice.piano_strike.clamp(0.01, 0.5); // hammer position along the string
            let bright = 0.45 + 0.55 * note.gain; // velocity opens the high partials
            let hammer = voice.piano_hammer.max(1e-3); // hardness: flattens the tilt
            let detune = 1.0 + (1.000_6_f32 - 1.0) * voice.piano_detune; // unison spread
            let string_det = [1.0 / detune, detune];

            let mut partials: Vec<Partial> = Vec::new();
            let mut k = 1usize;
            while k <= 18 {
                let kf = k as f32;
                let ratio = kf * (1.0 + b_inharm * kf * kf).sqrt();
                if ratio * f0 > 0.45 * srf {
                    break; // keep every partial below Nyquist
                }
                let notch = (std::f32::consts::PI * kf * strike).sin().abs();
                let amp = notch / kf * bright.powf((kf - 1.0) * 0.18 / hammer);
                let decay = (base_decay / (1.0 + 0.55 * (kf - 1.0))).max(0.05);
                partials.push(Partial {
                    step: ratio,
                    amp,
                    env: 1.0,
                    dmul: (-1.0 / (srf * decay)).exp(),
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
                        s += a * (TAU * *ph).sin();
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
        }
        SeqWave::Piano => {
            // Two strings detuned ±1.6 cents beat slowly against each other —
            // the chorusing shimmer of a real unison pair. Natural decay time
            // falls with pitch: bass strings ring for seconds, treble dies
            // in under one.
            let decay = (8.0 / (1.0 + f[0].max(20.0) / 110.0)).clamp(0.25, 6.0);
            let detune = 1.000_92; // 2^(1.6/1200)
            let (mut cph, mut mph) = (0.0f32, 0.0f32);
            let (mut cph2, mut mph2) = (0.0f32, 0.0f32);
            for (i, &fi) in f.iter().enumerate() {
                let dt = fi.max(0.0) / srf;
                let t = i as f32 / srf;
                // Hammer-strike brightness: louder keys strike brighter and
                // the shimmer fades within ~80 ms.
                let idx = (1.2 + 2.3 * note.gain) * (-t / 0.08).exp();
                let a = (TAU * cph + idx * (TAU * mph).sin()).sin();
                let b = (TAU * cph2 + idx * (TAU * mph2).sin()).sin();
                cph += dt / detune;
                cph -= cph.floor();
                mph += dt / detune;
                mph -= mph.floor();
                cph2 += dt * detune;
                cph2 -= cph2.floor();
                mph2 += dt * detune;
                mph2 -= mph2.floor();
                // Felt-hammer thump: 4 ms of soft noise on the attack.
                let thump = if t < 0.004 {
                    rng.bi() * 0.25 * (1.0 - t / 0.004)
                } else {
                    0.0
                };
                out.push((0.5 * (a + b) + thump) * (-t / decay).exp());
            }
        }
        SeqWave::Epiano => {
            // Rhodes-style: a soft FM body (1:1) under a metal tine (14:1)
            // that pings on the attack. Velocity opens the tine.
            let decay = (5.0 / (1.0 + f[0].max(20.0) / 250.0)).clamp(0.3, 4.0);
            let (mut cph, mut mph, mut tph) = (0.0f32, 0.0f32, 0.0f32);
            for (i, &fi) in f.iter().enumerate() {
                let dt = fi.max(0.0) / srf;
                let t = i as f32 / srf;
                let body_idx = (0.5 + 1.0 * note.gain) * (-t / 0.5).exp();
                let tine_idx = (0.8 + 1.4 * note.gain) * (-t / 0.035).exp();
                let body = (TAU * cph + body_idx * (TAU * mph).sin()).sin();
                let tine = (TAU * cph + tine_idx * (TAU * tph).sin()).sin();
                cph += dt;
                cph -= cph.floor();
                mph += dt;
                mph -= mph.floor();
                tph += dt * 14.0;
                tph -= tph.floor();
                out.push((0.75 * body + 0.25 * tine) * (-t / decay).exp());
            }
        }
        SeqWave::Organ => {
            // Tonewheel drawbars over half the fundamental (so the 16′ bar is
            // an integer partial and every phase wraps cleanly): 16′ 8′ 4′
            // 2⅔′ 2′, plus the classic percussion ping on the attack.
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
                    s += g * (TAU * phase * k).sin();
                }
                // Percussion: a 3rd-harmonic ping that fades in 200 ms.
                s += 0.5 * (-t / 0.2).exp() * (TAU * phase * 6.0).sin();
                out.push(s * norm);
                phase += fi.max(0.0) / 2.0 / srf;
                // Wrap on the full drawbar cycle to keep precision.
                phase -= phase.floor();
            }
        }
        SeqWave::Strings => {
            // Ensemble: three saws detuned ±8 cents, phase-spread, swelling
            // in like a bow stroke, mellowed by a one-pole lowpass.
            let detunes = [0.995_39f32, 1.0, 1.004_63]; // ∓8 cents
            let mut phases = [0.0f32, 0.33, 0.67];
            let lp_a = 1.0 - (-TAU * 3_000.0 / srf).exp();
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
                let swell = 1.0 - (-t / 0.12).exp();
                out.push(lp * swell);
            }
        }
        SeqWave::Bass => {
            // A saw through a velocity-swept one-pole lowpass over a sine sub.
            // Every constant is a `bass_*` knob; the defaults reproduce the
            // original fingered bass bit-for-bit (and draw no RNG, so it streams
            // byte-identically). `bass_click` adds a deterministic pick tick,
            // `bass_drive` a tanh grit, `bass_sub_ratio` an octave-down sub.
            const BASS_CLICK_TAU: f32 = 0.008;
            let decay = voice.bass_decay.max(1e-3);
            let body_decay = voice.bass_body_decay.max(1e-3);
            let drive = voice.bass_drive.clamp(0.0, 1.0);
            let mut phase = 0.0f32;
            let mut sub_phase = 0.0f32;
            let mut lp = 0.0f32;
            for (i, &fi) in f.iter().enumerate() {
                let dt = fi.max(0.0) / srf;
                let t = i as f32 / srf;
                let saw = (2.0 * phase - 1.0) - poly_blep(phase, dt);
                let cutoff = voice.bass_cutoff
                    + (voice.bass_env + voice.bass_env_vel * note.gain) * (-t / decay).exp()
                    + voice.bass_click * (-t / BASS_CLICK_TAU).exp();
                let a = 1.0 - (-TAU * cutoff / srf).exp();
                lp += a * (saw - lp);
                let body = lp + drive * ((lp * (1.0 + 2.0 * drive)).tanh() - lp);
                let sub = (TAU * sub_phase).sin();
                out.push((voice.bass_body * body + voice.bass_sub * sub) * (-t / body_decay).exp());
                phase += dt;
                phase -= phase.floor();
                sub_phase += dt * voice.bass_sub_ratio;
                sub_phase -= sub_phase.floor();
            }
        }
        SeqWave::Kit => out = kit_drum(f, note, sr, rng, voice.kit),
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
        let key = (69.0 + 12.0 * (hz / 440.0).log2()).round() as i32;
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
#[cfg(feature = "sampler")]
fn load_soundfont(path: &str) -> anyhow::Result<std::sync::Arc<rustysynth::SoundFont>> {
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex, OnceLock};
    static CACHE: OnceLock<Mutex<HashMap<String, Arc<rustysynth::SoundFont>>>> = OnceLock::new();
    let cache = CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    if let Some(f) = cache.lock().unwrap_or_else(|e| e.into_inner()).get(path) {
        return Ok(f.clone());
    }
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
