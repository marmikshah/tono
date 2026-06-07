//! Deterministic graph → samples renderer.
//!
//! Rendering is a pure function of `(graph, seed, sample_rate)`. Each node is
//! evaluated into a block of `f32` samples; combinators combine those blocks.
//! Processors transform the signal flowing through a `chain`.

use crate::analysis;
use crate::dsl::{
    Adsr, Curve, DriveShape, Modulator, Node, NoiseColor, Normalize, Playback, SeqNote, SeqWave,
    Shape, SoundDoc, Stereo, SuperWave, Value,
};
use crate::dsp::{Rng, db_to_lin, peak_limit};
use std::f32::consts::{FRAC_PI_2, TAU};

/// A block of mono audio samples.
type Signal = Vec<f32>;

/// Render a `tracks` document to a finished stereo pair: each track is
/// rendered mono and equal-power panned onto the bus (sampler tracks keep
/// their native stereo), the master chain runs per channel (the reverb with
/// decorrelated tails), then loop/normalize apply jointly.
pub fn render_tracks(doc: &SoundDoc) -> Option<(Signal, Signal)> {
    let Node::Tracks { tracks, master } = &doc.root else {
        return None;
    };
    let sr = doc.sample_rate;
    let n = ((doc.duration * sr as f32).ceil() as usize).max(1);
    let mut rng = Rng::new(doc.seed);
    let (mut left, mut right) = (vec![0.0f32; n], vec![0.0f32; n]);
    for t in tracks {
        // Equal-power pan law.
        let theta = (t.pan.clamp(-1.0, 1.0) + 1.0) * std::f32::consts::FRAC_PI_4;
        let (gl, gr) = (theta.cos() * t.gain, theta.sin() * t.gain);
        if let Some((l, r)) = track_native_stereo(&t.node, n, sr) {
            // A sampler track keeps its recorded stereo image; pan biases it.
            for i in 0..n {
                left[i] += l[i] * gl * std::f32::consts::SQRT_2;
                right[i] += r[i] * gr * std::f32::consts::SQRT_2;
            }
        } else {
            let mono = render_node(&t.node, n, sr, &mut rng);
            for (i, x) in mono.into_iter().enumerate() {
                left[i] += x * gl;
                right[i] += x * gr;
            }
        }
    }
    // Master bus: run each processor on both channels with identical state
    // seeds (the rng is cloned so e.g. a duck trigger fires identically), and
    // give the reverb the classic Freeverb stereo spread for a wide tail.
    for m in master {
        if let Node::Reverb { room, mix } = m {
            left = reverb(&left, *room, *mix, sr, 0);
            right = reverb(&right, *room, *mix, sr, 23);
        } else {
            let mut rl = rng.clone();
            left = apply_processor(m, &left, sr, &mut rl);
            right = apply_processor(m, &right, sr, &mut rng);
        }
    }
    if let Playback::Loop {
        start_secs,
        end_secs,
        crossfade_secs,
    } = doc.playback
    {
        left = make_loop_buffer(&left, sr, start_secs, end_secs, crossfade_secs);
        right = make_loop_buffer(&right, sr, start_secs, end_secs, crossfade_secs);
    }
    if let Some(nz) = &doc.normalize {
        normalize_output(&mut left, nz);
        normalize_output(&mut right, nz);
    }
    peak_limit(&mut [&mut left, &mut right]);
    Some((left, right))
}

/// A track whose node is directly a sampler seq renders in native stereo.
fn track_native_stereo(node: &Node, n: usize, sr: u32) -> Option<(Signal, Signal)> {
    if let Node::Seq {
        bpm,
        steps_per_beat,
        wave: SeqWave::Sampler,
        duty,
        fm_ratio,
        fm_index,
        fm_strike,
        pluck_decay,
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
            wave: SeqWave::Sampler,
            duty,
            fm_ratio: *fm_ratio,
            fm_index: *fm_index,
            fm_strike: *fm_strike,
            pluck_decay: *pluck_decay,
            sf2,
            sf2_preset: *sf2_preset,
            sf2_bank: *sf2_bank,
            swing: *swing,
            humanize: *humanize,
            env,
        };
        let step_dur = sr as f32 * 60.0 / bpm / (*steps_per_beat).max(1) as f32;
        return sampler_seq_stereo(&voice, notes, step_dur, n, sr);
    }
    None
}

/// Render a sound document to normalized mono samples in [-1, 1].
pub fn render(doc: &SoundDoc) -> Signal {
    if let Some((l, r)) = render_tracks(doc) {
        // Mono consumers (analysis, mono export) get the mid signal.
        return l.iter().zip(r).map(|(a, b)| 0.5 * (a + b)).collect();
    }
    let sr = doc.sample_rate;
    let n = ((doc.duration * sr as f32).ceil() as usize).max(1);
    let mut rng = Rng::new(doc.seed);
    let mut out = render_node(&doc.root, n, sr, &mut rng);
    // A loop is rendered as its seamless body (tail crossfaded onto the head).
    if let Playback::Loop {
        start_secs,
        end_secs,
        crossfade_secs,
    } = doc.playback
    {
        out = make_loop_buffer(&out, sr, start_secs, end_secs, crossfade_secs);
    }
    match &doc.normalize {
        // Loudness-matched / true-peak-limited output stage (opt-in).
        Some(nz) => normalize_output(&mut out, nz),
        // Default: a transparent sample-peak safety limit only.
        None => peak_limit(&mut [&mut out]),
    }
    out
}

/// Extract the loop region `[start_secs, end_secs)` and equal-power crossfade
/// its last `crossfade_secs` onto its head, returning a buffer that repeats
/// seamlessly. The output length is the region minus the crossfade.
///
/// Overlap-loop: with region `r` of length `L` and crossfade `x`, the body is
/// `r[0..L-x]` with its first `x` samples replaced by a sin/cos blend of the
/// head (`r[i]`, fading in) and the tail (`r[L-x+i]`, fading out). The wrap
/// `out[last] → out[0]` then lands on adjacent original samples, so there is no
/// discontinuity.
pub fn make_loop_buffer(
    samples: &[f32],
    sr: u32,
    start_secs: f32,
    end_secs: Option<f32>,
    crossfade_secs: f32,
) -> Signal {
    let len = samples.len();
    let s = ((start_secs * sr as f32) as usize).min(len);
    let e = end_secs
        .map(|x| (x * sr as f32) as usize)
        .unwrap_or(len)
        .min(len);
    if e <= s {
        return samples.to_vec();
    }
    let region = &samples[s..e];
    let l = region.len();
    let x = ((crossfade_secs * sr as f32) as usize).min(l / 2);
    if x == 0 {
        return region.to_vec();
    }
    let out_len = l - x;
    let mut out = region[..out_len].to_vec();
    for (i, o) in out.iter_mut().take(x).enumerate() {
        let t = (i as f32 + 0.5) / x as f32;
        let fade_in = (FRAC_PI_2 * t).sin();
        let fade_out = (FRAC_PI_2 * t).cos();
        *o = region[i] * fade_in + region[out_len + i] * fade_out;
    }
    out
}

/// The loop-seam discontinuity in dB: the sample jump from the last sample back
/// to the first (lower ⇒ a cleaner seamless loop).
pub fn loop_seam_db(samples: &[f32]) -> f32 {
    if samples.len() < 2 {
        return -120.0;
    }
    let jump = (samples[0] - samples[samples.len() - 1]).abs();
    20.0 * jump.max(1e-9).log10()
}

/// Opt-in output stage: gain-match to a LUFS target (if given), soft-limiting
/// peaks into the `ceiling_dbtp` true-peak ceiling. Unlike a whole-buffer
/// attenuation, the soft-knee limiter only compresses the peaks, so dense /
/// peaky material (a BGM mix, layered impacts) actually REACHES the loudness
/// target instead of being dragged back down. Two measure→gain→limit passes
/// converge within ~1 dB.
fn normalize_output(samples: &mut [f32], nz: &Normalize) {
    let ceil = db_to_lin(nz.ceiling_dbtp);
    if let Some(target) = nz.target_lufs {
        for _ in 0..2 {
            let cur = analysis::loudness_lufs(samples);
            if cur <= -120.0 {
                break;
            }
            let g = db_to_lin(target - cur);
            for x in samples.iter_mut() {
                *x *= g;
            }
            soft_limit(samples, ceil);
        }
    }
    // Safety: catch inter-sample residue above the ceiling, then sample peak.
    true_peak_limit(samples, nz.ceiling_dbtp);
    peak_limit(&mut [samples]);
}

/// Soft-knee peak limiter: transparent below `0.7 × ceil`, smoothly (tanh)
/// compressed above, never exceeding `ceil`. C1-continuous at the knee.
fn soft_limit(samples: &mut [f32], ceil: f32) {
    const KNEE: f32 = 0.7;
    for x in samples.iter_mut() {
        let v = *x / ceil;
        let a = v.abs();
        if a > KNEE {
            let compressed = KNEE + (1.0 - KNEE) * ((a - KNEE) / (1.0 - KNEE)).tanh();
            *x = v.signum() * compressed * ceil;
        }
    }
}

/// Scale so the estimated true peak sits at or below `ceiling_dbtp`. Pure
/// attenuation (never boosts), so it composes after loudness matching.
fn true_peak_limit(samples: &mut [f32], ceiling_dbtp: f32) {
    let ceil = db_to_lin(ceiling_dbtp);
    let tp = analysis::true_peak(samples);
    if tp > ceil && tp > 0.0 {
        let g = ceil / tp;
        for x in samples.iter_mut() {
            *x *= g;
        }
    }
}

/// Turn a finished mono render into a stereo (left, right) pair per the doc's
/// [`Stereo`] mode. Mono is the identity; Haas / Wide add width. The pair is
/// jointly peak-limited so widening never clips.
pub fn stereoize(mono: &[f32], stereo: Stereo, sr: u32) -> (Vec<f32>, Vec<f32>) {
    let (mut l, mut r) = match stereo {
        Stereo::Mono => (mono.to_vec(), mono.to_vec()),
        Stereo::Haas { ms, pan } => {
            let d = ((ms / 1000.0) * sr as f32) as usize;
            let delayed: Vec<f32> = (0..mono.len())
                .map(|i| if i >= d { mono[i - d] } else { 0.0 })
                .collect();
            // pan >= 0 → right leads (left is the delayed/trailing side).
            if pan >= 0.0 {
                (delayed, mono.to_vec())
            } else {
                (mono.to_vec(), delayed)
            }
        }
        Stereo::Wide { amount } => {
            let dec = allpass_decorrelate(mono, sr);
            let a = amount.clamp(0.0, 1.0);
            let mut l = Vec::with_capacity(mono.len());
            let mut r = Vec::with_capacity(mono.len());
            for i in 0..mono.len() {
                let mid = mono[i];
                let side = a * (mono[i] - dec[i]) * 0.5;
                l.push(mid + side);
                r.push(mid - side);
            }
            (l, r)
        }
    };
    peak_limit(&mut [&mut l, &mut r]);
    (l, r)
}

/// Decorrelate a mono signal with a short Schroeder all-pass chain (for the
/// `Wide` pseudo-stereo mode).
fn allpass_decorrelate(input: &[f32], sr: u32) -> Vec<f32> {
    let scale = sr as f32 / 44_100.0;
    let mut sig = input.to_vec();
    for &tune in &[225usize, 556, 441] {
        let len = ((tune as f32 * scale) as usize).max(1);
        let mut buf = vec![0.0f32; len];
        let mut idx = 0usize;
        let g = 0.7;
        for s in sig.iter_mut() {
            let buffered = buf[idx];
            let y = -*s * g + buffered;
            buf[idx] = *s + buffered * g;
            idx = (idx + 1) % len;
            *s = y;
        }
    }
    sig
}

/// Evaluate a parameter into a per-sample buffer of length `n`.
fn eval_value(v: &Value, n: usize, sr: u32) -> Vec<f32> {
    let srf = sr as f32;
    match v {
        Value::Const(c) => vec![*c; n],
        Value::Note(s) => vec![crate::dsl::note_to_hz(s).unwrap_or(440.0); n],
        Value::Modulated(Modulator::Slide {
            from,
            to,
            secs,
            curve,
        }) => (0..n)
            .map(|i| {
                let t = i as f32 / srf;
                let p = (t / secs).clamp(0.0, 1.0);
                match curve {
                    Curve::Lin => from + (to - from) * p,
                    Curve::Exp if *from > 0.0 && *to > 0.0 => {
                        // Geometric interpolation (natural for pitch / cutoff).
                        from * (to / from).powf(p)
                    }
                    Curve::Exp => {
                        // Fall back to an eased curve when values cross zero.
                        let e = p * p;
                        from + (to - from) * e
                    }
                }
            })
            .collect(),
        Value::Modulated(Modulator::Lfo {
            shape,
            rate,
            depth,
            center,
        }) => (0..n)
            .map(|i| {
                let phase = (i as f32 / srf * rate).fract();
                center + depth * osc(*shape, phase)
            })
            .collect(),
        Value::Modulated(Modulator::Arp { steps, rate }) => (0..n)
            .map(|i| {
                let t = i as f32 / srf;
                let idx = (t * rate) as usize % steps.len();
                steps[idx]
            })
            .collect(),
        Value::Modulated(Modulator::EnvMod {
            adsr: env,
            from,
            to,
        }) => {
            let e = adsr(env, n, sr);
            e.iter().map(|x| from + (to - from) * x).collect()
        }
    }
}

/// PolyBLEP residual for band-limited oscillators: corrects the discontinuity
/// at a phase edge to suppress aliasing. `t` is the phase (0..1), `dt` the
/// per-sample phase increment.
fn poly_blep(mut t: f32, dt: f32) -> f32 {
    if dt <= 0.0 {
        return 0.0;
    }
    if t < dt {
        t /= dt;
        t + t - t * t - 1.0
    } else if t > 1.0 - dt {
        t = (t - 1.0) / dt;
        t * t + t + t + 1.0
    } else {
        0.0
    }
}

/// Unit-amplitude oscillator value in [-1, 1] for a phase in [0, 1).
fn osc(shape: Shape, phase: f32) -> f32 {
    match shape {
        Shape::Sine => (TAU * phase).sin(),
        Shape::Square => {
            if phase < 0.5 {
                1.0
            } else {
                -1.0
            }
        }
        Shape::Triangle => {
            if phase < 0.5 {
                4.0 * phase - 1.0
            } else {
                3.0 - 4.0 * phase
            }
        }
        Shape::Saw => 2.0 * phase - 1.0,
    }
}

/// Render a node into a signal of length `n`.
fn render_node(node: &Node, n: usize, sr: u32, rng: &mut Rng) -> Signal {
    match node {
        Node::Square { freq, duty } => square_signal(freq, duty, n, sr),
        Node::Triangle { freq } => tri_signal(freq, n, sr),
        Node::Sawtooth { freq } => saw_signal(freq, n, sr),
        Node::Super {
            wave,
            freq,
            voices,
            detune_cents,
        } => super_signal(*wave, freq, *voices, *detune_cents, n, sr),
        Node::Sine { freq } => osc_signal(freq, n, sr, |p| osc(Shape::Sine, p)),
        Node::Noise { color } => noise_signal(*color, n, rng),
        Node::Fm { freq, ratio, index } => fm_signal(freq, *ratio, index, n, sr),
        Node::Seq {
            bpm,
            steps_per_beat,
            wave,
            duty,
            fm_ratio,
            fm_index,
            fm_strike,
            pluck_decay,
            sf2,
            sf2_preset,
            sf2_bank,
            swing,
            humanize,
            env,
            notes,
        } => {
            let voice = SeqVoice {
                wave: *wave,
                duty,
                fm_ratio: *fm_ratio,
                fm_index: *fm_index,
                fm_strike: *fm_strike,
                pluck_decay: *pluck_decay,
                sf2,
                sf2_preset: *sf2_preset,
                sf2_bank: *sf2_bank,
                swing: *swing,
                humanize: *humanize,
                env,
            };
            render_seq(*bpm, *steps_per_beat, &voice, notes, n, sr, rng)
        }
        Node::Env { adsr: env } => adsr(env, n, sr),
        // Validation rejects nested mixers; render defensively as a plain sum.
        Node::Tracks { tracks, .. } => {
            let mut acc = vec![0.0f32; n];
            for t in tracks {
                let sig = render_node(&t.node, n, sr, rng);
                for (o, v) in acc.iter_mut().zip(sig) {
                    *o += v * t.gain;
                }
            }
            acc
        }
        Node::Mix { inputs } => {
            let mut acc = vec![0.0f32; n];
            for input in inputs {
                let s = render_node(input, n, sr, rng);
                for (o, v) in acc.iter_mut().zip(s) {
                    *o += v;
                }
            }
            acc
        }
        Node::Mul { inputs } => {
            let mut acc = vec![1.0f32; n];
            for input in inputs {
                let s = render_node(input, n, sr, rng);
                for (o, v) in acc.iter_mut().zip(s) {
                    *o *= v;
                }
            }
            acc
        }
        Node::Chain { stages } => {
            let mut buf: Option<Signal> = None;
            for stage in stages {
                buf = Some(match (&buf, stage.is_processor()) {
                    // A processor transforms the running signal.
                    (Some(input), true) => apply_processor(stage, input, sr, rng),
                    // A source/combinator as a later stage replaces the signal.
                    (_, _) => render_node(stage, n, sr, rng),
                });
            }
            buf.unwrap_or_else(|| vec![0.0; n])
        }
        // A processor rendered standalone (outside a chain) has no input ⇒ silence.
        _ if node.is_processor() => vec![0.0; n],
        // Every non-processor variant is matched above; this fires only if a
        // new source is added to the DSL without a render arm.
        _ => unreachable!("unhandled source node in render_node"),
    }
}

/// Apply a processor node to an incoming signal. (`rng` feeds processors that
/// render an internal side signal, e.g. `duck`'s trigger.)
fn apply_processor(node: &Node, input: &[f32], sr: u32, rng: &mut Rng) -> Signal {
    match node {
        Node::Duck {
            trigger,
            amount,
            attack,
            release,
        } => {
            // Render the trigger silently; its loudness envelope steers a
            // gain dip on the chained signal — the sidechain pump.
            let trig = render_node(trigger, input.len(), sr, rng);
            let srf = sr as f32;
            let at = (-1.0 / (attack.max(1e-4) * srf)).exp();
            let rt = (-1.0 / (release.max(1e-4) * srf)).exp();
            let mut env = 0.0f32;
            input
                .iter()
                .zip(trig)
                .map(|(&x, t)| {
                    let rect = t.abs().min(1.0);
                    let coeff = if rect > env { at } else { rt };
                    env = rect + coeff * (env - rect);
                    x * (1.0 - amount * env)
                })
                .collect()
        }
        Node::Lowpass { cutoff, q } => biquad(input, cutoff, *q, sr, FilterKind::Low),
        Node::Highpass { cutoff, q } => biquad(input, cutoff, *q, sr, FilterKind::High),
        Node::Bandpass { cutoff, q } => biquad(input, cutoff, *q, sr, FilterKind::Band),
        Node::Notch { cutoff, q } => biquad(input, cutoff, *q, sr, FilterKind::Notch),
        Node::Peak { cutoff, q, gain_db } => {
            biquad(input, cutoff, *q, sr, FilterKind::Peak(*gain_db))
        }
        Node::Lowshelf { cutoff, gain_db } => {
            biquad(input, cutoff, 0.707, sr, FilterKind::LowShelf(*gain_db))
        }
        Node::Highshelf { cutoff, gain_db } => {
            biquad(input, cutoff, 0.707, sr, FilterKind::HighShelf(*gain_db))
        }
        Node::Gain { amount } => {
            let g = eval_value(amount, input.len(), sr);
            input.iter().zip(g).map(|(x, k)| x * k).collect()
        }
        Node::Bitcrush { bits } => {
            let levels = (1u32 << *bits as u32) as f32;
            let half = levels / 2.0;
            input
                .iter()
                .map(|x| (x.clamp(-1.0, 1.0) * half).round() / half)
                .collect()
        }
        Node::Downsample { factor } => {
            let f = (*factor).max(1) as usize;
            let mut out = Vec::with_capacity(input.len());
            let mut held = 0.0;
            for (i, &x) in input.iter().enumerate() {
                if i % f == 0 {
                    held = x;
                }
                out.push(held);
            }
            out
        }
        Node::Delay { secs, feedback } => {
            let dn = ((secs * sr as f32) as usize).max(1);
            let mut buf = vec![0.0f32; dn];
            let mut w = 0usize;
            let mut out = Vec::with_capacity(input.len());
            for &x in input {
                let delayed = buf[w];
                let y = x + feedback * delayed;
                buf[w] = y;
                w = (w + 1) % dn;
                out.push(y);
            }
            out
        }
        Node::Reverb { room, mix } => reverb(input, *room, *mix, sr, 0),
        Node::Drive { amount, shape } => {
            let a = eval_value(amount, input.len(), sr);
            input
                .iter()
                .zip(a)
                .map(|(x, amt)| drive_curve(amt.max(0.0) * x, *shape))
                .collect()
        }
        Node::RingMod { freq } => {
            let f = eval_value(freq, input.len(), sr);
            let srf = sr as f32;
            let mut phase = 0.0f32;
            let mut out = Vec::with_capacity(input.len());
            for (i, &x) in input.iter().enumerate() {
                out.push(x * (TAU * phase).sin());
                phase += f[i].max(0.0) / srf;
                phase -= phase.floor();
            }
            out
        }
        Node::Chorus { rate, depth, mix } => chorus(input, *rate, *depth, *mix, sr),
        Node::Flanger {
            rate,
            depth,
            feedback,
            mix,
        } => flanger(input, *rate, *depth, *feedback, *mix, sr),
        Node::Phaser {
            rate,
            depth,
            feedback,
            mix,
        } => phaser(input, *rate, *depth, *feedback, *mix, sr),
        Node::Compress {
            threshold,
            ratio,
            attack,
            release,
            makeup,
        } => compress(input, *threshold, *ratio, *attack, *release, *makeup, sr),
        _ => input.to_vec(),
    }
}

#[derive(Clone, Copy)]
enum FilterKind {
    Low,
    High,
    Band,
    Notch,
    /// Peaking EQ, gain in dB.
    Peak(f32),
    /// Low shelf, gain in dB.
    LowShelf(f32),
    /// High shelf, gain in dB.
    HighShelf(f32),
}

/// RBJ biquad with per-sample coefficient updates so the cutoff can be
/// modulated. State carried in Direct Form I. Peaking/shelving kinds carry a
/// dB gain (`A = 10^(gain/40)`).
fn biquad(input: &[f32], cutoff: &Value, q: f32, sr: u32, kind: FilterKind) -> Signal {
    let fc = eval_value(cutoff, input.len(), sr);
    let srf = sr as f32;
    let q = q.max(0.05);
    let nyq = srf / 2.0;
    let amp = match kind {
        FilterKind::Peak(g) | FilterKind::LowShelf(g) | FilterKind::HighShelf(g) => {
            10f32.powf(g / 40.0)
        }
        _ => 1.0,
    };
    let (mut x1, mut x2, mut y1, mut y2) = (0.0f32, 0.0f32, 0.0f32, 0.0f32);
    let mut out = Vec::with_capacity(input.len());
    for (i, &x0) in input.iter().enumerate() {
        let f = fc[i].clamp(20.0, nyq - 100.0);
        let w0 = TAU * f / srf;
        let (sin, cos) = w0.sin_cos();
        let alpha = sin / (2.0 * q);
        let (b0, b1, b2, a0, a1, a2) = match kind {
            FilterKind::Low => (
                (1.0 - cos) / 2.0,
                1.0 - cos,
                (1.0 - cos) / 2.0,
                1.0 + alpha,
                -2.0 * cos,
                1.0 - alpha,
            ),
            FilterKind::High => (
                (1.0 + cos) / 2.0,
                -(1.0 + cos),
                (1.0 + cos) / 2.0,
                1.0 + alpha,
                -2.0 * cos,
                1.0 - alpha,
            ),
            FilterKind::Band => (alpha, 0.0, -alpha, 1.0 + alpha, -2.0 * cos, 1.0 - alpha),
            FilterKind::Notch => (1.0, -2.0 * cos, 1.0, 1.0 + alpha, -2.0 * cos, 1.0 - alpha),
            FilterKind::Peak(_) => (
                1.0 + alpha * amp,
                -2.0 * cos,
                1.0 - alpha * amp,
                1.0 + alpha / amp,
                -2.0 * cos,
                1.0 - alpha / amp,
            ),
            FilterKind::LowShelf(_) => {
                let s = 2.0 * amp.sqrt() * alpha;
                let (ap1, am1) = (amp + 1.0, amp - 1.0);
                (
                    amp * (ap1 - am1 * cos + s),
                    2.0 * amp * (am1 - ap1 * cos),
                    amp * (ap1 - am1 * cos - s),
                    ap1 + am1 * cos + s,
                    -2.0 * (am1 + ap1 * cos),
                    ap1 + am1 * cos - s,
                )
            }
            FilterKind::HighShelf(_) => {
                let s = 2.0 * amp.sqrt() * alpha;
                let (ap1, am1) = (amp + 1.0, amp - 1.0);
                (
                    amp * (ap1 + am1 * cos + s),
                    -2.0 * amp * (am1 + ap1 * cos),
                    amp * (ap1 + am1 * cos - s),
                    ap1 - am1 * cos + s,
                    2.0 * (am1 - ap1 * cos),
                    ap1 - am1 * cos - s,
                )
            }
        };
        let y0 = (b0 / a0) * x0 + (b1 / a0) * x1 + (b2 / a0) * x2 - (a1 / a0) * y1 - (a2 / a0) * y2;
        x2 = x1;
        x1 = x0;
        y2 = y1;
        y1 = y0;
        out.push(y0);
    }
    out
}

/// Schroeder reverb: parallel feedback combs into series allpasses. Tunings are
/// the classic Freeverb values, scaled to the sample rate.
fn reverb(input: &[f32], room: f32, mix: f32, sr: u32, spread: usize) -> Signal {
    let scale = sr as f32 / 44_100.0;
    let comb_tunings = [
        1116 + spread,
        1188 + spread,
        1277 + spread,
        1356 + spread,
        1422 + spread,
        1491 + spread,
    ];
    let allpass_tunings = [556 + spread, 441 + spread, 341 + spread, 225 + spread];
    let feedback = 0.7 + 0.28 * room.clamp(0.0, 1.0);
    let damp = 0.2;

    let mut wet = vec![0.0f32; input.len()];
    // Parallel combs (summed).
    for &tune in &comb_tunings {
        let len = ((tune as f32 * scale) as usize).max(1);
        let mut buf = vec![0.0f32; len];
        let mut idx = 0usize;
        let mut filter_store = 0.0f32;
        for (i, &x) in input.iter().enumerate() {
            let y = buf[idx];
            filter_store = y * (1.0 - damp) + filter_store * damp;
            buf[idx] = x + filter_store * feedback;
            idx = (idx + 1) % len;
            wet[i] += y;
        }
    }
    // Series allpasses.
    for &tune in &allpass_tunings {
        let len = ((tune as f32 * scale) as usize).max(1);
        let mut buf = vec![0.0f32; len];
        let mut idx = 0usize;
        let g = 0.5;
        for w in wet.iter_mut() {
            let buffered = buf[idx];
            let y = -*w + buffered;
            buf[idx] = *w + buffered * g;
            idx = (idx + 1) % len;
            *w = y;
        }
    }
    let mix = mix.clamp(0.0, 1.0);
    let comb_norm = 1.0 / comb_tunings.len() as f32;
    input
        .iter()
        .zip(wet)
        .map(|(dry, w)| dry * (1.0 - mix) + (w * comb_norm) * mix)
        .collect()
}

/// Apply a waveshaper curve to a single sample.
fn drive_curve(x: f32, shape: DriveShape) -> f32 {
    match shape {
        DriveShape::Tanh => x.tanh(),
        DriveShape::Hard => x.clamp(-1.0, 1.0),
        DriveShape::Fold => {
            // Reflect anything outside [-1, 1] back inward (wavefolding).
            let mut y = x;
            while !(-1.0..=1.0).contains(&y) {
                if y > 1.0 {
                    y = 2.0 - y;
                } else {
                    y = -2.0 - y;
                }
            }
            y
        }
    }
}

/// Chorus: a single voice of modulated delay mixed with the dry signal.
fn chorus(input: &[f32], rate: f32, depth: f32, mix: f32, sr: u32) -> Signal {
    let srf = sr as f32;
    let base = 0.015 * srf; // ~15 ms centre delay
    let swing = depth.clamp(0.0, 1.0) * 0.010 * srf; // up to ±10 ms
    let max_delay = (base + swing) as usize + 2;
    let mut buf = vec![0.0f32; max_delay];
    let mut w = 0usize;
    let mix = mix.clamp(0.0, 1.0);
    let mut out = Vec::with_capacity(input.len());
    for (i, &x) in input.iter().enumerate() {
        buf[w] = x;
        let lfo = (TAU * rate * i as f32 / srf).sin();
        let delay = base + swing * lfo;
        // Fractional read via linear interpolation.
        let read = w as f32 - delay;
        let read = read.rem_euclid(max_delay as f32);
        let i0 = read.floor() as usize % max_delay;
        let i1 = (i0 + 1) % max_delay;
        let frac = read - read.floor();
        let wet = buf[i0] * (1.0 - frac) + buf[i1] * frac;
        out.push(x * (1.0 - mix) + wet * mix);
        w = (w + 1) % max_delay;
    }
    out
}

/// Flanger: a 0.5–6 ms swept delay with feedback, mixed against the dry path.
fn flanger(input: &[f32], rate: f32, depth: f32, feedback: f32, mix: f32, sr: u32) -> Signal {
    let srf = sr as f32;
    let base = 0.0025 * srf; // 2.5 ms centre
    let swing = depth.clamp(0.0, 1.0) * 0.002 * srf; // up to ±2 ms
    let max_delay = (base + swing) as usize + 2;
    let mut buf = vec![0.0f32; max_delay];
    let mut w = 0usize;
    let fb = feedback.clamp(0.0, 0.95);
    let mix = mix.clamp(0.0, 1.0);
    let mut out = Vec::with_capacity(input.len());
    for (i, &x) in input.iter().enumerate() {
        let lfo = (TAU * rate * i as f32 / srf).sin();
        let delay = base + swing * lfo;
        let read = (w as f32 - delay).rem_euclid(max_delay as f32);
        let i0 = read.floor() as usize % max_delay;
        let i1 = (i0 + 1) % max_delay;
        let frac = read - read.floor();
        let wet = buf[i0] * (1.0 - frac) + buf[i1] * frac;
        buf[w] = x + wet * fb;
        w = (w + 1) % max_delay;
        out.push(x * (1.0 - mix) + wet * mix);
    }
    out
}

/// Phaser: four first-order all-pass stages with an LFO-swept coefficient and
/// feedback — swept spectral notches.
fn phaser(input: &[f32], rate: f32, depth: f32, feedback: f32, mix: f32, sr: u32) -> Signal {
    let srf = sr as f32;
    let fb = feedback.clamp(0.0, 0.95);
    let mix = mix.clamp(0.0, 1.0);
    let depth = depth.clamp(0.0, 1.0);
    let mut x1 = [0.0f32; 4];
    let mut y1 = [0.0f32; 4];
    let mut last_wet = 0.0f32;
    let mut out = Vec::with_capacity(input.len());
    for (i, &x) in input.iter().enumerate() {
        // Sweep the all-pass coefficient between ~0.15 and ~0.85.
        let lfo = 0.5 + 0.5 * (TAU * rate * i as f32 / srf).sin();
        let g = 0.15 + 0.7 * depth * lfo;
        let mut s = x + last_wet * fb;
        for k in 0..4 {
            let y = -g * s + x1[k] + g * y1[k];
            x1[k] = s;
            y1[k] = y;
            s = y;
        }
        last_wet = s;
        out.push(x * (1.0 - mix) + s * mix);
    }
    out
}

/// Feed-forward compressor with a peak-detector envelope follower.
fn compress(
    input: &[f32],
    threshold_db: f32,
    ratio: f32,
    attack: f32,
    release: f32,
    makeup_db: f32,
    sr: u32,
) -> Signal {
    let srf = sr as f32;
    let at = (-1.0 / (attack.max(1e-4) * srf)).exp();
    let rt = (-1.0 / (release.max(1e-4) * srf)).exp();
    let makeup = 10f32.powf(makeup_db / 20.0);
    let ratio = ratio.max(1.0);
    let mut env = 0.0f32; // envelope in linear amplitude
    let mut out = Vec::with_capacity(input.len());
    for &x in input {
        let rect = x.abs();
        // Attack when rising, release when falling.
        let coeff = if rect > env { at } else { rt };
        env = rect + coeff * (env - rect);
        let env_db = 20.0 * env.max(1e-9).log10();
        let gain_db = if env_db > threshold_db {
            -(env_db - threshold_db) * (1.0 - 1.0 / ratio)
        } else {
            0.0
        };
        let g = 10f32.powf(gain_db / 20.0);
        out.push(x * g * makeup);
    }
    out
}

/// Drive a phase accumulator at a (possibly modulated) frequency and map each
/// phase to a sample via `wave`.
fn osc_signal(freq: &Value, n: usize, sr: u32, wave: impl Fn(f32) -> f32) -> Signal {
    let f = eval_value(freq, n, sr);
    let srf = sr as f32;
    let mut phase = 0.0f32;
    let mut out = Vec::with_capacity(n);
    for &fi in f.iter() {
        out.push(wave(phase));
        phase += fi.max(0.0) / srf;
        phase -= phase.floor();
    }
    out
}

/// Band-limited square / pulse with a per-sample (modulatable) duty — PWM.
/// PolyBLEP corrects both the rising (phase 0) and falling (phase = duty) edges.
fn square_signal(freq: &Value, duty: &Value, n: usize, sr: u32) -> Signal {
    let f = eval_value(freq, n, sr);
    let d = eval_value(duty, n, sr);
    let srf = sr as f32;
    let mut phase = 0.0f32;
    let mut out = Vec::with_capacity(n);
    for i in 0..n {
        let duty = d[i].clamp(0.01, 0.99);
        let dt = f[i].max(0.0) / srf;
        let mut v = if phase < duty { 1.0 } else { -1.0 };
        v += poly_blep(phase, dt);
        v -= poly_blep((phase - duty + 1.0).fract(), dt);
        out.push(v);
        phase += dt;
        phase -= phase.floor();
    }
    out
}

/// Band-limited sawtooth (naive ramp minus a PolyBLEP at the wrap).
fn saw_signal(freq: &Value, n: usize, sr: u32) -> Signal {
    let f = eval_value(freq, n, sr);
    let srf = sr as f32;
    let mut phase = 0.0f32;
    let mut out = Vec::with_capacity(n);
    for &fi in f.iter() {
        let dt = fi.max(0.0) / srf;
        out.push((2.0 * phase - 1.0) - poly_blep(phase, dt));
        phase += dt;
        phase -= phase.floor();
    }
    out
}

/// Band-limited triangle: integrate a band-limited (PolyBLEP) square. A leaky
/// integrator removes DC drift. Clean at high pitch, unlike a naive triangle.
fn tri_signal(freq: &Value, n: usize, sr: u32) -> Signal {
    let f = eval_value(freq, n, sr);
    let srf = sr as f32;
    let mut phase = 0.0f32;
    let mut tri = 0.0f32;
    let mut out = Vec::with_capacity(n);
    for &fi in f.iter() {
        let dt = fi.max(0.0) / srf;
        // Band-limited square (duty 0.5): rising edge at 0, falling at 0.5.
        let mut sq = if phase < 0.5 { 1.0 } else { -1.0 };
        sq += poly_blep(phase, dt);
        sq -= poly_blep((phase + 0.5).fract(), dt);
        // Integrate (slope ±4/period ⇒ unit-amplitude triangle); leak out DC.
        tri = tri * 0.9995 + 4.0 * dt * sq;
        out.push(tri);
        phase += dt;
        phase -= phase.floor();
    }
    out
}

/// Unison super-oscillator: sum `voices` detuned band-limited saw/square copies,
/// phase-spread for width, scaled by 1/voices so the level stays bounded.
fn super_signal(
    wave: SuperWave,
    freq: &Value,
    voices: u32,
    detune_cents: f32,
    n: usize,
    sr: u32,
) -> Signal {
    let f = eval_value(freq, n, sr);
    let srf = sr as f32;
    let v = voices.clamp(1, 16);
    let mut out = vec![0.0f32; n];
    for k in 0..v {
        // Symmetric detune spread across [-detune, +detune] cents.
        let cents = if v == 1 {
            0.0
        } else {
            -detune_cents + 2.0 * detune_cents * (k as f32 / (v as f32 - 1.0))
        };
        let ratio = 2f32.powf(cents / 1200.0);
        let mut phase = k as f32 / v as f32; // decorrelate voice phases
        for (i, o) in out.iter_mut().enumerate() {
            let dt = (f[i].max(0.0) * ratio) / srf;
            let s = match wave {
                SuperWave::Sawtooth => (2.0 * phase - 1.0) - poly_blep(phase, dt),
                SuperWave::Square => {
                    let mut sq = if phase < 0.5 { 1.0 } else { -1.0 };
                    sq += poly_blep(phase, dt);
                    sq -= poly_blep((phase + 0.5).fract(), dt);
                    sq
                }
            };
            *o += s;
            phase += dt;
            phase -= phase.floor();
        }
    }
    let scale = 1.0 / v as f32;
    for o in out.iter_mut() {
        *o *= scale;
    }
    out
}

/// Generate `n` samples of coloured noise.
fn noise_signal(color: NoiseColor, n: usize, rng: &mut Rng) -> Signal {
    match color {
        NoiseColor::White => (0..n).map(|_| rng.bi()).collect(),
        NoiseColor::Pink => {
            // Paul Kellet's economical pink-noise filter.
            let (mut b0, mut b1, mut b2, mut b3, mut b4, mut b5, mut b6) =
                (0.0f32, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0);
            (0..n)
                .map(|_| {
                    let w = rng.bi();
                    b0 = 0.99886 * b0 + w * 0.0555179;
                    b1 = 0.99332 * b1 + w * 0.0750759;
                    b2 = 0.96900 * b2 + w * 0.153_852;
                    b3 = 0.86650 * b3 + w * 0.3104856;
                    b4 = 0.55000 * b4 + w * 0.5329522;
                    b5 = -0.7616 * b5 - w * 0.0168980;
                    let out = b0 + b1 + b2 + b3 + b4 + b5 + b6 + w * 0.5362;
                    b6 = w * 0.115926;
                    out * 0.11
                })
                .collect()
        }
        NoiseColor::Brown => {
            // Leaky integration of white noise.
            let mut last = 0.0f32;
            (0..n)
                .map(|_| {
                    last = (last + 0.02 * rng.bi()) * 0.998;
                    (last * 8.0).clamp(-1.0, 1.0)
                })
                .collect()
        }
    }
}

/// Two-operator FM: carrier phase modulated by an operator at `freq * ratio`.
fn fm_signal(freq: &Value, ratio: f32, index: &Value, n: usize, sr: u32) -> Signal {
    let f = eval_value(freq, n, sr);
    let idx = eval_value(index, n, sr);
    let srf = sr as f32;
    let (mut cph, mut mph) = (0.0f32, 0.0f32);
    let mut out = Vec::with_capacity(n);
    for i in 0..n {
        let m = idx[i] * (TAU * mph).sin();
        out.push((TAU * cph + m).sin());
        let fi = f[i].max(0.0);
        cph += fi / srf;
        cph -= cph.floor();
        mph += (fi * ratio) / srf;
        mph -= mph.floor();
    }
    out
}

/// The per-seq instrument settings shared by every note.
struct SeqVoice<'a> {
    wave: SeqWave,
    duty: &'a Value,
    fm_ratio: f32,
    fm_index: f32,
    fm_strike: f32,
    pluck_decay: f32,
    sf2: &'a str,
    sf2_preset: u32,
    sf2_bank: u32,
    swing: f32,
    humanize: f32,
    env: &'a Adsr,
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
        let mut hr = Rng::new((note.step as u64) << 32 ^ (note.len as u64) << 8 ^ 0x6A09_E667);
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
    if voice.wave == SeqWave::Sampler {
        return sampler_seq(voice, notes, step_dur, n, sr);
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
            // Karplus-Strong: a noise burst in a delay line tuned to the
            // note's onset pitch (a plucked string cannot glide). The
            // average-and-feed-back lowpass damps highs faster than lows,
            // exactly like a real string.
            let period = ((srf / f[0].clamp(20.0, srf / 2.0)).round() as usize).max(2);
            let mut string: Vec<f32> = (0..period).map(|_| rng.bi()).collect();
            let mut spos = 0usize;
            for _ in 0..n {
                let y = string[spos];
                let next = string[(spos + 1) % string.len()];
                string[spos] = voice.pluck_decay * 0.5 * (y + next);
                spos = (spos + 1) % string.len();
                out.push(y);
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
            // Fingered bass: a saw through a one-pole lowpass whose cutoff
            // snaps open with velocity and settles, over a sine sub.
            let mut phase = 0.0f32;
            let mut lp = 0.0f32;
            for (i, &fi) in f.iter().enumerate() {
                let dt = fi.max(0.0) / srf;
                let t = i as f32 / srf;
                let saw = (2.0 * phase - 1.0) - poly_blep(phase, dt);
                let cutoff = 250.0 + (700.0 + 1_100.0 * note.gain) * (-t / 0.15).exp();
                let a = 1.0 - (-TAU * cutoff / srf).exp();
                lp += a * (saw - lp);
                let sub = (TAU * phase).sin();
                out.push((0.7 * lp + 0.45 * sub) * (-t / 2.0).exp());
                phase += dt;
                phase -= phase.floor();
            }
        }
        SeqWave::Kit => out = kit_drum(f, note, sr, rng),
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

/// One sample of cowbell at fundamental `f`: two clashing partials (the
/// classic ~1.56 ratio of an 808 cowbell), saturated square-ish, with a fast
/// knock decay.
fn cowbell_sample(f: f32, t: f32) -> f32 {
    let a = (2.5 * (TAU * f * t).sin()).tanh();
    let b = (2.5 * (TAU * f * 1.565 * t).sin()).tanh();
    0.5 * (a + b) * (-t / 0.09).exp()
}

/// Render a whole sampler seq through rustysynth: real recorded instruments
/// from a SoundFont. All notes share one synthesizer so polyphony, voice
/// stealing, and per-preset envelopes behave like a real MIDI instrument.
/// Output is the stereo render downmixed to the graph's mono bus (doc-level
/// `stereo` adds width back at the output stage).
fn sampler_seq(voice: &SeqVoice, notes: &[SeqNote], step_dur: f32, n: usize, sr: u32) -> Signal {
    match sampler_seq_stereo(voice, notes, step_dur, n, sr) {
        Some((l, r)) => l.iter().zip(r).map(|(a, b)| 0.5 * (a + b)).collect(),
        None => vec![0.0; n],
    }
}

/// The sampler's native stereo render (used directly by mixer tracks).
fn sampler_seq_stereo(
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
fn load_soundfont(path: &str) -> anyhow::Result<std::sync::Arc<rustysynth::SoundFont>> {
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex, OnceLock};
    static CACHE: OnceLock<Mutex<HashMap<String, Arc<rustysynth::SoundFont>>>> = OnceLock::new();
    let cache = CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    if let Some(f) = cache.lock().unwrap().get(path) {
        return Ok(f.clone());
    }
    let mut file = std::fs::File::open(path)?;
    let font = Arc::new(
        rustysynth::SoundFont::new(&mut file).map_err(|e| anyhow::anyhow!("parse: {e:?}"))?,
    );
    cache.lock().unwrap().insert(path.to_string(), font.clone());
    Ok(font)
}

/// One General-MIDI-mapped drum hit: the note's onset pitch picks the voice.
fn kit_drum(f: &[f32], _note: &SeqNote, sr: u32, rng: &mut Rng) -> Signal {
    let srf = sr as f32;
    let n = f.len();
    // Recover the MIDI number from the onset frequency (pitch is wire-encoded
    // as Hz; "midi:36" round-trips exactly).
    let midi = (69.0 + 12.0 * (f[0].max(8.0) / 440.0).log2()).round() as i32;
    let mut out = Vec::with_capacity(n);

    // One-pole highpass state for cymbal/snare noise.
    let (mut lp, hp_a) = (0.0f32, 1.0 - (-TAU * 5_500.0 / srf).exp());
    let hp = |x: f32, lp: &mut f32| {
        *lp += hp_a * (x - *lp);
        x - *lp
    };
    let mut phase = 0.0f32;

    for i in 0..n {
        let t = i as f32 / srf;
        let s = match midi {
            // Kick: a fast downward pitch thump plus a 2 ms beater click.
            35 | 36 => {
                let fk = 45.0 + 105.0 * (-t / 0.04).exp();
                phase += fk / srf;
                phase -= phase.floor();
                let click = if t < 0.002 { rng.bi() * 0.4 } else { 0.0 };
                (TAU * phase).sin() * (-t / 0.13).exp() + click
            }
            // Snare / rimshot / clap: tone crack + noise body.
            38 | 40 => {
                let tone = (TAU * 190.0 * t).sin() * 0.4 * (-t / 0.06).exp();
                tone + rng.bi() * 0.8 * (-t / 0.11).exp()
            }
            37 => (TAU * 800.0 * t).sin() * 0.3 * (-t / 0.03).exp() + rng.bi() * (-t / 0.025).exp(),
            39 => rng.bi() * (-t / 0.09).exp(),
            // Hats: highpassed noise, closed dies fast, open rings.
            42 | 44 => hp(rng.bi(), &mut lp) * (-t / 0.035).exp(),
            46 => hp(rng.bi(), &mut lp) * (-t / 0.22).exp(),
            // Toms: pitched thumps falling with the GM map.
            41 | 43 | 45 | 47 | 48 | 50 => {
                let base = 80.0 + 24.0 * (midi - 41) as f32;
                let ft = base * (1.0 - 0.15 * (t / 0.2).min(1.0));
                phase += ft / srf;
                phase -= phase.floor();
                (TAU * phase).sin() * (-t / 0.18).exp() + rng.bi() * 0.1 * (-t / 0.03).exp()
            }
            // Cowbell (more cowbell).
            56 => cowbell_sample(540.0, t),
            // Crash / ride.
            49 | 55 | 57 => hp(rng.bi(), &mut lp) * (-t / 0.7).exp(),
            51 | 53 | 59 => {
                hp(rng.bi(), &mut lp) * 0.5 * (-t / 0.45).exp()
                    + (TAU * 5_200.0 * t).sin() * 0.25 * (-t / 0.25).exp()
            }
            // Anything unmapped: a generic percussive hit.
            _ => rng.bi() * (-t / 0.08).exp(),
        };
        out.push(s);
    }
    out
}

/// ADSR envelope with an sfxr-style punch boost on the initial transient.
fn adsr(env: &Adsr, n: usize, sr: u32) -> Signal {
    let Adsr { a, d, s, r, punch } = *env;
    let srf = sr as f32;
    let dur = n as f32 / srf;
    let rel_start = (dur - r).max(0.0);
    let punch_win = a + d;
    (0..n)
        .map(|i| {
            let t = i as f32 / srf;
            let mut v = if t < a {
                if a > 0.0 { t / a } else { 1.0 }
            } else if t < a + d {
                let p = if d > 0.0 { (t - a) / d } else { 1.0 };
                1.0 - (1.0 - s) * p
            } else if t < rel_start {
                s
            } else if r > 0.0 {
                let p = ((t - rel_start) / r).clamp(0.0, 1.0);
                s * (1.0 - p)
            } else {
                0.0
            };
            if punch > 0.0 && punch_win > 0.0 && t < punch_win {
                v *= 1.0 + punch * (1.0 - t / punch_win);
            }
            v
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn doc(json: &str) -> SoundDoc {
        serde_json::from_str(json).expect("deserialize")
    }

    fn rms(s: &[f32]) -> f32 {
        (s.iter().map(|x| x * x).sum::<f32>() / s.len() as f32).sqrt()
    }

    #[test]
    fn render_is_deterministic() {
        let d = doc(r#"{ "name": "n", "duration": 0.1, "seed": 7,
                 "root": { "type": "noise" } }"#);
        assert_eq!(render(&d), render(&d)); // same graph + seed ⇒ same bytes
        let mut d2 = d.clone();
        d2.seed = 8;
        assert_ne!(render(&d), render(&d2)); // different seed ⇒ different noise
    }

    #[test]
    fn sine_has_expected_length_and_level() {
        let d = doc(r#"{ "name": "n", "duration": 0.1,
                 "root": { "type": "sine", "freq": 440 } }"#);
        let s = render(&d);
        assert_eq!(s.len(), 4410);
        let peak = s.iter().fold(0.0f32, |m, &x| m.max(x.abs()));
        assert!(peak > 0.9 && peak <= 1.0);
        assert!((rms(&s) - 0.7).abs() < 0.05); // sine RMS = peak/√2
    }

    #[test]
    fn envelope_gates_the_oscillator() {
        let d = doc(
            r#"{ "name": "n", "duration": 0.2, "root": { "type": "mul", "inputs": [
                { "type": "square", "freq": 440 },
                { "type": "env", "a": 0.0, "d": 0.05, "s": 0.0, "r": 0.0 }
            ] } }"#,
        );
        let s = render(&d);
        // Loud at the start, silent at the end (envelope fully decayed).
        let head = rms(&s[..2205]);
        let tail = rms(&s[s.len() - 2205..]);
        assert!(head > 0.1, "head should be audible, rms {head}");
        assert!(tail < 1e-3, "tail should be silent, rms {tail}");
    }

    #[test]
    fn slide_descends_pitch() {
        // A 880→110 Hz exponential slide: zero crossings in the first half
        // outnumber those in the second half.
        let d = doc(r#"{ "name": "n", "duration": 0.5, "root": { "type": "sine",
                 "freq": { "slide": { "from": 880, "to": 110, "secs": 0.5, "curve": "exp" } } } }"#);
        let s = render(&d);
        let crossings = |w: &[f32]| w.windows(2).filter(|p| p[0] * p[1] < 0.0).count();
        let (a, b) = s.split_at(s.len() / 2);
        assert!(crossings(a) > crossings(b) * 2);
    }

    #[test]
    fn seq_places_notes_on_the_grid() {
        // 120 bpm, 4 steps/beat ⇒ 0.125 s per step. A note at step 2 starts at
        // 0.25 s; everything before is silence.
        let d = doc(r#"{ "name": "n", "duration": 0.6, "root": { "type": "seq",
                 "bpm": 120, "wave": "square",
                 "env": { "d": 0.1 },
                 "notes": [ { "step": 2, "len": 2, "pitch": "C4" } ] } }"#);
        let s = render(&d);
        let pre = rms(&s[..(0.24 * 44_100.0) as usize]);
        let post = rms(&s[(0.26 * 44_100.0) as usize..(0.35 * 44_100.0) as usize]);
        assert!(pre < 1e-4, "before the note: silence, rms {pre}");
        assert!(post > 0.05, "during the note: audible, rms {post}");
    }

    /// Brightness proxy: energy of the first difference relative to the
    /// signal (high-frequency content differentiates to larger steps).
    fn brightness(s: &[f32]) -> f32 {
        let diff: f32 = s.windows(2).map(|w| (w[1] - w[0]).powi(2)).sum();
        let total: f32 = s.iter().map(|x| x * x).sum();
        diff / total.max(1e-12)
    }

    #[test]
    fn lowpass_darkens_highpass_brightens() {
        let noise = r#"{ "type": "noise" }"#;
        let plain = doc(&format!(
            r#"{{ "name": "n", "duration": 0.2, "root": {noise} }}"#
        ));
        let lp = doc(&format!(
            r#"{{ "name": "n", "duration": 0.2, "root": {{ "type": "chain", "stages": [
                {noise}, {{ "type": "lowpass", "cutoff": 500 }} ] }} }}"#
        ));
        let hp = doc(&format!(
            r#"{{ "name": "n", "duration": 0.2, "root": {{ "type": "chain", "stages": [
                {noise}, {{ "type": "highpass", "cutoff": 5000 }} ] }} }}"#
        ));
        let b_plain = brightness(&render(&plain));
        assert!(brightness(&render(&lp)) < b_plain * 0.5, "lowpass darkens");
        assert!(
            brightness(&render(&hp)) > b_plain * 1.1,
            "highpass brightens"
        );
    }

    #[test]
    fn chain_processors_transform_in_series() {
        // sine → gain 0.25: the processor scales the running signal.
        let d = doc(
            r#"{ "name": "n", "duration": 0.05, "root": { "type": "chain", "stages": [
                { "type": "sine", "freq": 440 },
                { "type": "gain", "amount": 0.25 }
            ] } }"#,
        );
        let s = render(&d);
        let peak = s.iter().fold(0.0f32, |m, &x| m.max(x.abs()));
        assert!((peak - 0.25).abs() < 0.01);
    }

    #[test]
    fn bitcrush_quantizes_amplitude() {
        // The gain stage keeps the crushed peak under the output ceiling so the
        // safety limit stays out of the way and the levels survive untouched.
        let d = doc(
            r#"{ "name": "n", "duration": 0.05, "root": { "type": "chain", "stages": [
                { "type": "sine", "freq": 100 },
                { "type": "gain", "amount": 0.5 },
                { "type": "bitcrush", "bits": 2 }
            ] } }"#,
        );
        let s = render(&d);
        // 2 bits ⇒ amplitudes land on multiples of 0.5.
        for x in &s {
            let nearest = (x / 0.5).round() * 0.5;
            assert!((x - nearest).abs() < 1e-4, "{x} not on a 2-bit level");
        }
    }

    #[test]
    fn drive_hard_clips_to_unit_range() {
        let d = doc(
            r#"{ "name": "n", "duration": 0.05, "root": { "type": "chain", "stages": [
                { "type": "sine", "freq": 440 },
                { "type": "drive", "amount": 10, "shape": "hard" }
            ] } }"#,
        );
        // Heavy drive into a hard clip ⇒ near-square at the ceiling.
        let s = render(&d);
        let clipped = s.iter().filter(|x| x.abs() > 0.95).count();
        assert!(clipped > s.len() / 2);
    }

    #[test]
    fn compressor_attenuates_above_threshold() {
        // A 0 dBFS sine through threshold −20 dB, ratio 4:1 settles at a steady
        // gain of −(0 − (−20))·(1 − 1/4) = −15 dB.
        let wet = doc(
            r#"{ "name": "n", "duration": 0.3, "root": { "type": "chain", "stages": [
                { "type": "sine", "freq": 440 },
                { "type": "compress", "threshold": -20, "ratio": 4 }
            ] } }"#,
        );
        let dry =
            doc(r#"{ "name": "n", "duration": 0.3, "root": { "type": "sine", "freq": 440 } }"#);
        // Skip the attack transient, measure the settled tail.
        let tail = |s: Vec<f32>| rms(&s[s.len() / 2..]);
        let ratio = tail(render(&wet)) / tail(render(&dry));
        let db = 20.0 * ratio.log10();
        assert!((db + 15.0).abs() < 2.0, "expected ≈ −15 dB, got {db:.1} dB");
    }

    #[test]
    fn loop_body_is_region_minus_crossfade() {
        let sr = 1000u32;
        let samples = vec![0.5f32; 1000]; // 1 s
        // Region [0.2, 0.8) = 600 samples, crossfade 0.1 s = 100 ⇒ body 500.
        let out = make_loop_buffer(&samples, sr, 0.2, Some(0.8), 0.1);
        assert_eq!(out.len(), 500);
        // Degenerate inputs fall back gracefully.
        assert_eq!(
            make_loop_buffer(&samples, sr, 0.9, Some(0.1), 0.1).len(),
            1000
        );
        assert_eq!(make_loop_buffer(&samples, sr, 0.0, None, 0.0).len(), 1000);
    }

    #[test]
    fn looped_render_has_a_quiet_seam() {
        // A sustained noise bed rendered as a loop: the wrap-around jump should
        // be far below the raw signal's sample-to-sample movement.
        let d = doc(r#"{ "name": "n", "duration": 1.0, "seed": 3,
                 "playback": { "mode": "loop", "crossfade_secs": 0.25 },
                 "root": { "type": "chain", "stages": [
                    { "type": "noise" }, { "type": "lowpass", "cutoff": 800 } ] } }"#);
        let s = render(&d);
        assert!(s.len() < 44_100); // body shortened by the crossfade
        assert!(loop_seam_db(&s) < -20.0, "seam {} dB", loop_seam_db(&s));
    }

    #[test]
    fn normalize_hits_the_loudness_target() {
        let d = doc(r#"{ "name": "n", "duration": 0.5,
                 "normalize": { "target_lufs": -20, "ceiling_dbtp": -1 },
                 "root": { "type": "chain", "stages": [
                    { "type": "sine", "freq": 440 }, { "type": "gain", "amount": 0.05 } ] } }"#);
        let s = render(&d);
        let lufs = analysis::loudness_lufs(&s);
        assert!((lufs + 20.0).abs() < 1.5, "got {lufs} LUFS");
        // True peak respects the −1 dBTP ceiling (small estimation slack).
        assert!(crate::dsp::dbfs(analysis::true_peak(&s)) <= -0.9);
    }

    #[test]
    fn stereoize_modes_behave() {
        let d = doc(r#"{ "name": "n", "duration": 0.1, "root": { "type": "noise" } }"#);
        let mono = render(&d);
        let (l, r) = stereoize(&mono, Stereo::Mono, 44_100);
        assert_eq!(l, r);
        let (l, r) = stereoize(&mono, Stereo::Wide { amount: 0.8 }, 44_100);
        assert_ne!(l, r); // decorrelated channels differ...
        let mid_rms = rms(&l
            .iter()
            .zip(&r)
            .map(|(a, b)| (a + b) / 2.0)
            .collect::<Vec<_>>());
        assert!(mid_rms > 0.1); // ...but the mid (mono sum) survives
        let (l, r) = stereoize(&mono, Stereo::Haas { ms: 10.0, pan: 1.0 }, 44_100);
        let delay = (0.010 * 44_100.0) as usize;
        assert_eq!(l[delay..delay + 100], mono[..100]); // left trails by 10 ms
        assert_eq!(r[..100], mono[..100]); // right leads
    }

    #[test]
    fn fm_seq_strikes_bright_then_mellows() {
        // One sustained fm note: the decaying modulation index makes the
        // attack brighter than the tail — the hammer-strike signature.
        let d = doc(r#"{ "name": "n", "duration": 1.0, "root": { "type": "seq",
                 "bpm": 60, "steps_per_beat": 1, "wave": "fm",
                 "fm_ratio": 1.0, "fm_index": 6, "fm_strike": 0.15,
                 "env": { "d": 0.9, "s": 0.5 },
                 "notes": [ { "step": 0, "len": 1, "pitch": "A3" } ] } }"#);
        let s = render(&d);
        assert!(rms(&s) > 0.05, "fm note audible");
        let third = s.len() / 3;
        assert!(
            brightness(&s[..third]) > brightness(&s[2 * third..]) * 1.5,
            "strike should be brighter than the tail"
        );
    }

    #[test]
    fn pluck_seq_rings_and_decays_deterministically() {
        let json = r#"{ "name": "n", "duration": 1.2, "seed": 9, "root": { "type": "seq",
            "bpm": 60, "steps_per_beat": 1, "wave": "pluck", "pluck_decay": 0.995,
            "env": { "d": 0.1, "s": 1.0 },
            "notes": [ { "step": 0, "len": 1, "pitch": "A3" } ] } }"#;
        let s = render(&doc(json));
        let half = s.len() / 2;
        assert!(rms(&s[..half]) > 0.05, "pluck audible");
        assert!(
            rms(&s[half..]) < rms(&s[..half]) * 0.5,
            "string decays naturally"
        );
        // Same seed ⇒ identical string; different seed ⇒ different noise burst.
        assert_eq!(s, render(&doc(json)));
        let mut other = doc(json);
        other.seed = 10;
        assert_ne!(s, render(&other));
    }

    #[test]
    fn piano_bass_rings_longer_than_treble() {
        let note = |pitch: &str| {
            let d = doc(&format!(
                r#"{{ "name": "n", "duration": 2.0, "root": {{ "type": "seq",
                     "bpm": 60, "steps_per_beat": 1, "wave": "piano",
                     "env": {{ "a": 0.002, "s": 1.0, "r": 0.1 }},
                     "notes": [ {{ "step": 0, "len": 2, "pitch": "{pitch}" }} ] }} }}"#
            ));
            render(&d)
        };
        let tail_ratio = |s: &[f32]| {
            let q = s.len() / 4;
            rms(&s[2 * q..3 * q]) / rms(&s[..q]).max(1e-9)
        };
        let bass = note("A1");
        let treble = note("A5");
        assert!(rms(&bass) > 0.02 && rms(&treble) > 0.005, "both audible");
        assert!(
            tail_ratio(&bass) > tail_ratio(&treble) * 1.5,
            "bass sustains, treble dies: {} vs {}",
            tail_ratio(&bass),
            tail_ratio(&treble)
        );
    }

    fn one_note(wave: &str, pitch: &str, secs: f32) -> Vec<f32> {
        let d = doc(&format!(
            r#"{{ "name": "n", "duration": {secs}, "root": {{ "type": "seq",
                 "bpm": 60, "steps_per_beat": 1, "wave": "{wave}",
                 "env": {{ "a": 0.002, "s": 1.0, "r": 0.05 }},
                 "notes": [ {{ "step": 0, "len": {len}, "pitch": "{pitch}" }} ] }} }}"#,
            len = secs.ceil() as u32,
        ));
        render(&d)
    }

    #[test]
    fn epiano_tine_pings_then_mellows() {
        let s = one_note("epiano", "A3", 1.0);
        assert!(rms(&s) > 0.05, "epiano audible");
        let q = s.len() / 4;
        assert!(brightness(&s[..q]) > brightness(&s[3 * q..]) * 1.3);
    }

    #[test]
    fn organ_sustains_while_held() {
        let s = one_note("organ", "C3", 1.0);
        assert!(rms(&s) > 0.1, "organ audible");
        let q = s.len() / 4;
        // No natural decay: the last quarter holds level with the second.
        let (mid, tail) = (rms(&s[q..2 * q]), rms(&s[3 * q..]));
        assert!(tail > mid * 0.7, "organ holds: {mid} -> {tail}");
    }

    #[test]
    fn strings_swell_in_slowly() {
        let s = one_note("strings", "A3", 1.0);
        assert!(rms(&s) > 0.05, "strings audible");
        let ms50 = 44_100 / 20;
        // The bow swell: the first 50 ms is much quieter than the body.
        assert!(rms(&s[..ms50]) < rms(&s[ms50 * 6..ms50 * 8]) * 0.6);
    }

    #[test]
    fn bass_is_darker_than_a_raw_saw() {
        let b = one_note("bass", "E2", 0.5);
        let saw = one_note("sawtooth", "E2", 0.5);
        assert!(rms(&b) > 0.05, "bass audible");
        assert!(
            brightness(&b) < brightness(&saw) * 0.5,
            "bass is filtered dark"
        );
    }

    #[test]
    fn tracks_pan_places_instruments_on_the_stage() {
        let d = doc(
            r#"{ "name": "n", "duration": 0.2, "root": { "type": "tracks", "tracks": [
                { "pan": -1.0, "node": { "type": "sine", "freq": 440 } },
                { "pan":  1.0, "gain": 0.5, "node": { "type": "sine", "freq": 660 } }
            ] } }"#,
        );
        assert_eq!(d.validate(), Ok(()));
        let (l, r) = render_tracks(&d).unwrap();
        // Hard-left 440 dominates L; hard-right (at half gain) is alone on R.
        assert!(
            rms(&l) > rms(&r) * 1.5,
            "left louder: {} vs {}",
            rms(&l),
            rms(&r)
        );
        let zero_crossings = |s: &[f32]| s.windows(2).filter(|w| w[0] * w[1] < 0.0).count();
        // R carries only the 660 Hz track ⇒ more crossings per second.
        assert!(zero_crossings(&r) > zero_crossings(&l));
        // The public mono render is the mid of the same bus.
        let mid = render(&d);
        assert!((mid[1000] - 0.5 * (l[1000] + r[1000])).abs() < 1e-6);
    }

    #[test]
    fn tracks_master_reverb_decorrelates_the_channels() {
        let d = doc(
            r#"{ "name": "n", "duration": 0.5, "root": { "type": "tracks",
                 "tracks": [ { "node": { "type": "mul", "inputs": [
                     { "type": "sine", "freq": 440 },
                     { "type": "env", "d": 0.1 } ] } } ],
                 "master": [ { "type": "reverb", "room": 0.6, "mix": 0.4 } ] } }"#,
        );
        let (l, r) = render_tracks(&d).unwrap();
        assert_ne!(l, r, "spread reverb gives each side its own tail");
        // And with a duck in the master, both channels stay deterministic.
        let d2 = doc(
            r#"{ "name": "n", "duration": 0.5, "seed": 3, "root": { "type": "tracks",
                 "tracks": [ { "node": { "type": "noise" } } ],
                 "master": [ { "type": "duck", "amount": 0.7,
                   "trigger": { "type": "seq", "bpm": 120, "steps_per_beat": 1,
                     "wave": "kit", "env": { "s": 1 },
                     "notes": [ { "step": 0, "len": 1, "pitch": "midi:36" } ] } } ] } }"#,
        );
        let a = render_tracks(&d2).unwrap();
        let b = render_tracks(&d2).unwrap();
        assert_eq!(a, b, "stereo master bus renders are byte-stable");
    }

    #[test]
    fn tracks_validation_guards_the_console() {
        let nested = doc(r#"{ "name": "n", "root": { "type": "mix", "inputs": [
                { "type": "tracks", "tracks": [ { "node": { "type": "noise" } } ] }
            ] } }"#);
        assert!(nested.validate().unwrap_err().contains("root"));
        let bad_master = doc(r#"{ "name": "n", "root": { "type": "tracks",
                 "tracks": [ { "node": { "type": "noise" } } ],
                 "master": [ { "type": "sine", "freq": 440 } ] } }"#);
        assert!(bad_master.validate().unwrap_err().contains("master"));
        let bad_pan = doc(r#"{ "name": "n", "root": { "type": "tracks",
                 "tracks": [ { "pan": 2.0, "node": { "type": "noise" } } ] } }"#);
        assert!(bad_pan.validate().unwrap_err().contains("pan"));
    }

    #[test]
    fn sampler_requires_a_real_soundfont_path() {
        let d = doc(r#"{ "name": "n", "duration": 0.5, "root": { "type": "seq",
                 "bpm": 120, "wave": "sampler", "sf2": "/no/such/font.sf2",
                 "env": { "s": 1 },
                 "notes": [ { "step": 0, "len": 2, "pitch": "C4" } ] } }"#);
        assert!(d.validate().unwrap_err().contains("no such file"));
        let d = doc(r#"{ "name": "n", "duration": 0.5, "root": { "type": "seq",
                 "bpm": 120, "wave": "sampler",
                 "env": { "s": 1 },
                 "notes": [ { "step": 0, "len": 2, "pitch": "C4" } ] } }"#);
        assert!(d.validate().unwrap_err().contains("sf2"));
    }

    /// Full sampler audio check — needs a real SoundFont. Set
    /// SONARIUM_TEST_SF2=/path/to/any_gm_bank.sf2 to enable; skipped (and
    /// printed as such) otherwise so CI stays hermetic.
    #[test]
    fn sampler_renders_real_instruments_deterministically() {
        let Some(sf2) = std::env::var_os("SONARIUM_TEST_SF2") else {
            eprintln!("skipping sampler audio test: SONARIUM_TEST_SF2 not set");
            return;
        };
        let sf2 = sf2.to_string_lossy().replace('"', "");
        let d = doc(&format!(
            r#"{{ "name": "n", "duration": 2.0, "root": {{ "type": "seq",
                 "bpm": 120, "wave": "sampler", "sf2": "{sf2}", "sf2_preset": 0,
                 "env": {{ "s": 1 }},
                 "notes": [ {{ "step": 0, "len": 2, "pitch": "C4" }},
                            {{ "step": 2, "len": 2, "pitch": "E4" }},
                            {{ "step": 4, "len": 4, "pitch": "G4" }} ] }} }}"#
        ));
        let s = render(&d);
        assert!(rms(&s) > 0.01, "sampled piano audible");
        assert_eq!(s, render(&d), "sampler render is deterministic");
        // Percussion bank: a GM kick on channel 9.
        let k = doc(&format!(
            r#"{{ "name": "n", "duration": 1.0, "root": {{ "type": "seq",
                 "bpm": 120, "wave": "sampler", "sf2": "{sf2}", "sf2_bank": 128,
                 "env": {{ "s": 1 }},
                 "notes": [ {{ "step": 0, "len": 2, "pitch": "midi:36" }} ] }} }}"#
        ));
        assert!(rms(&render(&k)[..8820]) > 0.01, "sampled kick audible");
    }

    #[test]
    fn duck_pumps_a_pad_under_its_trigger() {
        // A steady pad ducked by a kick pattern: rms right after each kick is
        // lower than between kicks.
        let d = doc(
            r#"{ "name": "n", "duration": 1.0, "root": { "type": "chain", "stages": [
                { "type": "sine", "freq": 220 },
                { "type": "duck", "amount": 0.9, "release": 0.2,
                  "trigger": { "type": "seq", "bpm": 120, "steps_per_beat": 1,
                    "wave": "kit", "env": { "s": 1 },
                    "notes": [ { "step": 0, "len": 1, "pitch": "midi:36" },
                               { "step": 1, "len": 1, "pitch": "midi:36" } ] } }
            ] } }"#,
        );
        let s = render(&d);
        let sr = 44_100;
        // 60 ms right after the kick at t=0 vs the recovered region ~0.4 s.
        let after_kick = rms(&s[..sr * 6 / 100]);
        let recovered = rms(&s[sr * 2 / 5..sr * 45 / 100]);
        assert!(
            after_kick < recovered * 0.65,
            "pumped {after_kick} vs recovered {recovered}"
        );
    }

    #[test]
    fn swing_delays_offbeats_and_humanize_jitters_deterministically() {
        let beat = |extra: &str| {
            let d = doc(&format!(
                r#"{{ "name": "n", "duration": 1.0, "root": {{ "type": "seq",
                     "bpm": 120, "steps_per_beat": 2, "wave": "sine"{extra},
                     "env": {{ "d": 0.05 }},
                     "notes": [ {{ "step": 0, "len": 1, "pitch": 880 }},
                                {{ "step": 1, "len": 1, "pitch": 880 }} ] }} }}"#
            ));
            render(&d)
        };
        let onset =
            |s: &[f32], from: usize| from + s[from..].iter().position(|x| x.abs() > 0.05).unwrap();
        let straight = beat("");
        let swung = beat(r#", "swing": 0.6"#);
        // Step 1 (the off-beat, at 0.25 s) lands later when swung; step 0 doesn't.
        let half = 44_100 / 5; // search after 0.2 s
        assert_eq!(onset(&straight, 0), onset(&swung, 0));
        let (a, b) = (onset(&straight, half), onset(&swung, half));
        let expected = (0.6 * 0.5 * 0.25 * 44_100.0) as usize; // swing*half*step
        assert!(
            (b - a) as i64 - expected as i64 <= 2,
            "off-beat delayed by ~{expected}, got {}",
            b - a
        );
        // Humanize changes timing/level but is deterministic.
        let h1 = beat(r#", "humanize": 0.3"#);
        let h2 = beat(r#", "humanize": 0.3"#);
        assert_eq!(h1, h2);
        assert_ne!(h1, straight);
    }

    #[test]
    fn cowbell_knocks_and_tracks_pitch() {
        let lo = one_note("cowbell", "A4", 1.0);
        let hi = one_note("cowbell", "A5", 1.0);
        assert!(rms(&lo[..4410]) > 0.1, "cowbell knocks");
        assert!(brightness(&hi) > brightness(&lo), "pitch tracks the note");
        // Fast knock decay: the tail is near-silent.
        assert!(rms(&lo[lo.len() / 2..]) < 0.01);
        // And the kit's fixed cowbell (GM 56) responds too.
        let kit = one_note("kit", "midi:56", 0.3);
        assert!(rms(&kit[..4410]) > 0.05, "kit cowbell audible");
    }

    #[test]
    fn kit_maps_pitches_to_distinct_drums() {
        let kick = one_note("kit", "midi:36", 0.4);
        let snare = one_note("kit", "midi:38", 0.4);
        let hat = one_note("kit", "midi:42", 0.4);
        for (name, s) in [("kick", &kick), ("snare", &snare), ("hat", &hat)] {
            assert!(rms(s) > 0.01, "{name} audible");
        }
        // Spectral ordering: kick < snare < hat.
        assert!(brightness(&kick) < brightness(&snare));
        assert!(brightness(&snare) < brightness(&hat));
        // Hat dies fast; open hat (midi:46) rings longer.
        let open = one_note("kit", "midi:46", 0.4);
        let q = hat.len() / 4;
        assert!(rms(&open[q..2 * q]) > rms(&hat[q..2 * q]) * 2.0);
        // Noise-based drums stay deterministic.
        assert_eq!(snare, one_note("kit", "midi:38", 0.4));
    }

    #[test]
    fn seq_with_absurd_note_lengths_stays_bounded() {
        // A 4-billion-step note and a near-zero bpm must not allocate
        // note-length buffers beyond the render window (OOM guard).
        let d = doc(r#"{ "name": "n", "duration": 0.1, "root": { "type": "seq",
                 "bpm": 120, "wave": "square", "env": { "d": 0.05 },
                 "notes": [ { "step": 0, "len": 4000000000, "pitch": 440 } ] } }"#);
        assert_eq!(render(&d).len(), 4410);
        let d = doc(r#"{ "name": "n", "duration": 0.1, "root": { "type": "seq",
                 "bpm": 0.0001, "wave": "sine", "env": { "d": 0.05 },
                 "notes": [ { "step": 0, "len": 1, "pitch": 440 } ] } }"#);
        assert_eq!(render(&d).len(), 4410);
    }

    #[test]
    fn mix_layers_and_mul_gates() {
        let d = doc(
            r#"{ "name": "n", "duration": 0.05, "root": { "type": "mix", "inputs": [
                { "type": "sine", "freq": 220 },
                { "type": "sine", "freq": 330 }
            ] } }"#,
        );
        assert!(rms(&render(&d)) > 0.5);
        let d = doc(
            r#"{ "name": "n", "duration": 0.05, "root": { "type": "mul", "inputs": [
                { "type": "sine", "freq": 220 },
                { "type": "gain", "amount": 1 }
            ] } }"#,
        );
        // gain (a processor) standalone renders silence; mul with silence = silence.
        assert!(rms(&render(&d)) < 1e-6);
    }
}
