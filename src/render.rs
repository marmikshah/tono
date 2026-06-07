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

/// Render a sound document to normalized mono samples in [-1, 1].
pub fn render(doc: &SoundDoc) -> Signal {
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
                env,
            };
            render_seq(*bpm, *steps_per_beat, &voice, notes, n, sr, rng)
        }
        Node::Env { adsr: env } => adsr(env, n, sr),
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
                    (Some(input), true) => apply_processor(stage, input, sr),
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

/// Apply a processor node to an incoming signal.
fn apply_processor(node: &Node, input: &[f32], sr: u32) -> Signal {
    match node {
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
        Node::Reverb { room, mix } => reverb(input, *room, *mix, sr),
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
fn reverb(input: &[f32], room: f32, mix: f32, sr: u32) -> Signal {
    let scale = sr as f32 / 44_100.0;
    let comb_tunings = [1116, 1188, 1277, 1356, 1422, 1491];
    let allpass_tunings = [556, 441, 341, 225];
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
    env: &'a Adsr,
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
    let mut out = vec![0.0f32; n];
    for note in notes {
        let start = (note.step as f32 * step_dur) as usize;
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
            out[start + i] += s * envb[i] * note.gain;
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
            for i in 0..n {
                let dt = f[i].max(0.0) / srf;
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
            for i in 0..n {
                let dt = f[i].max(0.0) / srf;
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
