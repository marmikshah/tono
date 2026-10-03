//! Filters and effect processors: the biquad family, reverb, convolution, the
//! granular texture, the waveshaper (with ADAA), the modal resonator bank,
//! modulation effects, and dynamics.

use super::{Signal, eval_value};
use crate::dsl::{DriveShape, Mode, Value};
use crate::dsp::{self, Rng};
use std::f32::consts::{LN_2, PI, TAU};

#[derive(Clone, Copy)]
pub(crate) enum FilterKind {
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

/// Shared deterministic RBJ coefficients `(b0, b1, b2, a1, a2)` for a cutoff.
/// A live cutoff sweep recomputes these same coefficients.
pub(crate) fn biquad_coeffs(
    kind: FilterKind,
    fc: f32,
    q: f32,
    sr: u32,
) -> (f32, f32, f32, f32, f32) {
    let srf = sr as f32;
    let q = q.max(0.05);
    let nyq = srf / 2.0;
    // `.max(20.0)` keeps the clamp bounds ordered at absurdly low sample
    // rates (an unvalidated doc must not panic).
    let f = fc.clamp(20.0, (nyq - 100.0).max(20.0));
    let w0 = TAU * f / srf;
    let (sin, cos) = dsp::sin_cos(w0);
    let alpha = sin / (2.0 * q);
    let amp = match kind {
        FilterKind::Peak(g) | FilterKind::LowShelf(g) | FilterKind::HighShelf(g) => {
            dsp::powf(10.0, g / 40.0)
        }
        _ => 1.0,
    };
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
    (b0 / a0, b1 / a0, b2 / a0, a1 / a0, a2 / a0)
}

/// Apply a filter or EQ with a constant or modulated cutoff.
pub(super) fn biquad(input: &[f32], cutoff: &Value, q: f32, sr: u32, kind: FilterKind) -> Signal {
    let fc = eval_value(cutoff, input.len(), sr);
    let (mut x1, mut x2, mut y1, mut y2) = (0.0f32, 0.0f32, 0.0f32, 0.0f32);
    let mut out = Vec::with_capacity(input.len());
    // Coefficients are a pure function of the cutoff (kind/q/sr fixed), so
    // recompute only when it moves — for the common constant/piecewise-constant
    // cutoff this hoists the transcendental work out of the loop entirely,
    // with bit-identical coefficients (and output) either way.
    let mut prev_f = f32::NAN;
    let (mut b0, mut b1, mut b2, mut a1, mut a2) = (0.0f32, 0.0f32, 0.0f32, 0.0f32, 0.0f32);
    for (i, &x0) in input.iter().enumerate() {
        if fc[i] != prev_f {
            (b0, b1, b2, a1, a2) = biquad_coeffs(kind, fc[i], q, sr);
            prev_f = fc[i];
        }
        let y0 = b0 * x0 + b1 * x1 + b2 * x2 - a1 * y1 - a2 * y2;
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
pub(super) fn reverb(input: &[f32], room: f32, mix: f32, sr: u32, spread: usize) -> Signal {
    let (comb_lens, allpass_lens) = crate::dsp::freeverb_lengths(sr, spread);
    let feedback = crate::dsp::freeverb_feedback(room);
    let damp = crate::dsp::FREEVERB_DAMP;

    let mut wet = vec![0.0f32; input.len()];
    // Parallel combs (summed).
    for &len in &comb_lens {
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
    for &len in &allpass_lens {
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
    let comb_norm = 1.0 / comb_lens.len() as f32;
    input
        .iter()
        .zip(wet)
        .map(|(dry, w)| dry * (1.0 - mix) + (w * comb_norm) * mix)
        .collect()
}

/// Apply a waveshaper curve to one sample.
pub(crate) fn drive_curve(x: f32, shape: DriveShape) -> f32 {
    match shape {
        DriveShape::Tanh => dsp::tanh(x),
        DriveShape::Hard => x.clamp(-1.0, 1.0),
        DriveShape::Fold => {
            // Reflect anything outside [-1, 1] back inward (wavefolding).
            // This runs per sample on the real-time path, so it must always
            // terminate: a non-finite input would otherwise loop forever, and
            // the iteration cap bounds pathological amplitudes (folding is
            // musically meaningless that far out; no sane input gets near it).
            if !x.is_finite() {
                return 0.0;
            }
            let mut y = x;
            let mut folds = 0;
            while !(-1.0..=1.0).contains(&y) {
                if y > 1.0 {
                    y = 2.0 - y;
                } else {
                    y = -2.0 - y;
                }
                folds += 1;
                if folds > 1024 {
                    return y.clamp(-1.0, 1.0);
                }
            }
            y
        }
    }
}

/// Antiderivative of the waveshaper, used by ADAA as `F(x1) − F(x0)`.
pub(crate) fn drive_antideriv(x: f32, shape: DriveShape) -> f32 {
    match shape {
        // ∫ tanh = ln(cosh x). Computed as |x| + ln(1+e^{−2|x|}) − ln 2 so it
        // never overflows for large |x| (cosh would).
        DriveShape::Tanh => {
            let a = x.abs();
            a + dsp::ln_1p(dsp::exp(-2.0 * a)) - LN_2
        }
        // ∫ clamp(x,−1,1): x²/2 inside the linear region, |x|−1/2 outside
        // (continuous at ±1, both give 1/2).
        DriveShape::Hard => {
            let a = x.abs();
            if a <= 1.0 { 0.5 * x * x } else { a - 0.5 }
        }
        // The fold is a period-4 triangle wave; its antiderivative is the
        // continuous, period-4 piecewise parabola below (zero-mean ⇒ bounded,
        // so it is safe for arbitrarily large |x|). Reduce x into one period
        // first: p = (x+1) mod 4 ∈ [0,4).
        DriveShape::Fold => {
            let p = (x + 1.0).rem_euclid(4.0);
            if p <= 2.0 {
                0.5 * (p - 1.0) * (p - 1.0)
            } else {
                1.0 - 0.5 * (p - 3.0) * (p - 3.0)
            }
        }
    }
}

/// First-order antiderivative anti-aliasing for the memoryless waveshaper.
///
/// A pointwise nonlinearity sprays harmonics past Nyquist that fold back as
/// inharmonic "digital" grit. ADAA replaces `f(x)` with the average of `f`
/// over `[x[n-1], x[n]]` — `(F(x[n]) − F(x[n-1])) / (x[n] − x[n-1])` — which
/// band-limits the result, suppressing the foldback. The `f(midpoint)`
/// fallback avoids the 0/0 (and its catastrophic cancellation) when
/// consecutive inputs are nearly equal. One sample of state is carried across
/// the block. A one-pole DC blocker follows: the difference-quotient leaves a
/// small DC term on asymmetric input.
pub(super) fn drive_adaa(input: &[f32], amount: &[f32], shape: DriveShape) -> Signal {
    const EPS: f32 = 1e-5;
    // ~5 Hz one-pole DC blocker (y[n] = x[n] − x[n−1] + R·y[n−1]).
    const R: f32 = 0.9995;
    let mut x_prev = 0.0f32;
    let mut f_prev = drive_antideriv(0.0, shape);
    let (mut dc_x, mut dc_y) = (0.0f32, 0.0f32);
    let mut out = Vec::with_capacity(input.len());
    for (&x, &amt) in input.iter().zip(amount) {
        let xn = amt.max(0.0) * x;
        let f = drive_antideriv(xn, shape);
        let d = xn - x_prev;
        let y = if d.abs() > EPS {
            (f - f_prev) / d
        } else {
            drive_curve(0.5 * (xn + x_prev), shape)
        };
        x_prev = xn;
        f_prev = f;
        let yb = y - dc_x + R * dc_y;
        dc_x = y;
        dc_y = yb;
        out.push(yb);
    }
    out
}

/// Modal resonator bank: sum of N parallel two-pole resonators driven by the
/// incoming signal. Each mode is a complex-conjugate pole pair at radius `r`
/// and angle `ω`: `y[n] = b0·x[n] + 2r·cos(ω)·y[n-1] − r²·y[n-2]`. The pole
/// radius sets the decay exactly — `r^(decay·sr) = 0.001`, i.e. −60 dB at the
/// mode's ring time — and `b0 = gain·sin(ω)` normalises the impulse-response
/// peak to `gain`, so a mode's loudness is its `gain` regardless of how long
/// it rings. Coefficients are constant per mode (LTI), so no per-sample
/// recompute and no zipper. Deterministic: pure f32 arithmetic, fixed
/// coefficients.
pub(super) fn modal_bank(input: &[f32], modes: &[Mode], mix: f32, sr: u32) -> Signal {
    let mix = mix.clamp(0.0, 1.0);
    let mut wet = vec![0.0f32; input.len()];
    for m in modes {
        let (a1, a2, b0) = crate::dsp::modal_coeffs(m.freq, m.decay, m.gain, sr);
        let (mut y1, mut y2) = (0.0f32, 0.0f32);
        for (o, &x) in wet.iter_mut().zip(input) {
            let y = b0 * x + a1 * y1 + a2 * y2;
            y2 = y1;
            y1 = y;
            *o += y;
        }
    }
    input
        .iter()
        .zip(wet)
        .map(|(d, w)| d * (1.0 - mix) + w * mix)
        .collect()
}

/// Chorus: a single voice of modulated delay mixed with the dry signal.
pub(super) fn chorus(input: &[f32], rate: f32, depth: f32, mix: f32, sr: u32) -> Signal {
    let srf = sr as f32;
    let base = crate::dsp::CHORUS_BASE_SECS * srf;
    let swing = depth.clamp(0.0, 1.0) * crate::dsp::CHORUS_SWING_SECS * srf;
    let max_delay = (base + swing) as usize + 2;
    let mut buf = vec![0.0f32; max_delay];
    let mut w = 0usize;
    let mix = mix.clamp(0.0, 1.0);
    let mut out = Vec::with_capacity(input.len());
    for (i, &x) in input.iter().enumerate() {
        buf[w] = x;
        let lfo = dsp::sin(TAU * rate * i as f32 / srf);
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
pub(super) fn flanger(
    input: &[f32],
    rate: f32,
    depth: f32,
    feedback: f32,
    mix: f32,
    sr: u32,
) -> Signal {
    let srf = sr as f32;
    let base = crate::dsp::FLANGER_BASE_SECS * srf; // 2.5 ms centre
    let swing = depth.clamp(0.0, 1.0) * crate::dsp::FLANGER_SWING_SECS * srf; // up to ±2 ms
    let max_delay = (base + swing) as usize + 2;
    let mut buf = vec![0.0f32; max_delay];
    let mut w = 0usize;
    let fb = feedback.clamp(0.0, 0.95);
    let mix = mix.clamp(0.0, 1.0);
    let mut out = Vec::with_capacity(input.len());
    for (i, &x) in input.iter().enumerate() {
        let lfo = dsp::sin(TAU * rate * i as f32 / srf);
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
pub(super) fn phaser(
    input: &[f32],
    rate: f32,
    depth: f32,
    feedback: f32,
    mix: f32,
    sr: u32,
) -> Signal {
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
        let lfo = 0.5 + 0.5 * dsp::sin(TAU * rate * i as f32 / srf);
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

/// The `compress` node's parameters, bundled so the processor function stays
/// under the arity cap (see [`ConvolveSpec`]).
#[derive(Clone, Copy)]
pub(super) struct CompressSpec {
    /// Threshold in dBFS.
    pub threshold: f32,
    /// Compression ratio (≥ 1).
    pub ratio: f32,
    /// Attack time in seconds.
    pub attack: f32,
    /// Release time in seconds.
    pub release: f32,
    /// Makeup gain in dB.
    pub makeup: f32,
}

/// Feed-forward compressor with a peak-detector envelope follower.
pub(super) fn compress(input: &[f32], spec: CompressSpec, sr: u32) -> Signal {
    let srf = sr as f32;
    let at = dsp::exp(-1.0 / (spec.attack.max(1e-4) * srf));
    let rt = dsp::exp(-1.0 / (spec.release.max(1e-4) * srf));
    let makeup = dsp::powf(10.0, spec.makeup / 20.0);
    let ratio = spec.ratio.max(1.0);
    let mut env = 0.0f32; // envelope in linear amplitude
    let mut out = Vec::with_capacity(input.len());
    for &x in input {
        let rect = x.abs();
        // Attack when rising, release when falling.
        let coeff = if rect > env { at } else { rt };
        env = rect + coeff * (env - rect);
        let env_db = 20.0 * dsp::log10(env.max(1e-9));
        let gain_db = if env_db > spec.threshold {
            -(env_db - spec.threshold) * (1.0 - 1.0 / ratio)
        } else {
            0.0
        };
        let g = dsp::powf(10.0, gain_db / 20.0);
        out.push(x * g * makeup);
    }
    out
}

/// The `convolve` node's parameters, bundled so the processor function stays
/// under the arity cap.
#[derive(Clone, Copy)]
pub(super) struct ConvolveSpec {
    /// RT60-ish decay time in seconds.
    pub decay: f32,
    /// IR length cap in seconds (0 = `decay`).
    pub size: f32,
    /// Pre-delay in seconds.
    pub predelay: f32,
    /// High-frequency damping, 0..1.
    pub damp: f32,
    /// Dry/wet mix, 0..1.
    pub mix: f32,
}

/// Synthesize the impulse response of a [`convolve`] node, deterministically
/// from `seed` (the node's structural stream key, so the same node at the same
/// graph position always builds the same IR — zero assets, no IR files):
/// a white-noise burst under an exponential envelope reaching −60 dB at
/// `decay` seconds, truncated at `size` (0 = `decay`), darkened over time by a
/// one-pole lowpass whose cutoff falls from ~0.45·sr toward 100 Hz as `damp`
/// goes 0 → 1, preceded by `predelay` seconds of silence. Normalized to unit
/// energy, so the wet level stays put as `decay`/`size` change.
fn synth_ir(decay: f32, size: f32, predelay: f32, damp: f32, sr: u32, seed: u64) -> Vec<f32> {
    let srf = sr as f32;
    // validate() caps all three at 30 s; the clamps guard a direct render of
    // an unvalidated doc from an unbounded allocation (delay.secs pattern).
    let decay = decay.clamp(1e-3, 30.0);
    let ir_secs = if size > 0.0 {
        size.clamp(1e-3, 30.0)
    } else {
        decay
    };
    let pre = (predelay.clamp(0.0, 30.0) * srf) as usize;
    let len = ((ir_secs * srf) as usize).max(1);
    let mut rng = Rng::new(seed);
    let mut ir = vec![0.0f32; pre + len];
    let mut lp = 0.0f32;
    for (i, s) in ir.iter_mut().skip(pre).enumerate() {
        let t = i as f32 / srf;
        let env = dsp::exp(crate::dsp::NEG_LN_1000 * t / decay);
        // The cutoff falls linearly across the tail as damp goes 0 → 1
        // (0 = the burst stays ~white, 1 = it closes to 100 Hz by the end).
        let fc = (0.45 * srf * (1.0 - damp.clamp(0.0, 1.0) * t / ir_secs)).max(100.0);
        let a = 1.0 - dsp::exp(-TAU * fc / srf);
        lp += a * (rng.bi() - lp);
        *s = lp * env;
    }
    let energy = ir.iter().map(|x| x * x).sum::<f32>().sqrt();
    if energy > 0.0 {
        for x in ir.iter_mut() {
            *x /= energy;
        }
    }
    ir
}

/// Offline convolution with the synthesized impulse response.
/// Uses the fixed radix-2 FFT in `det`, zero-padded to the next power of two
/// covering input + impulse − 1. Crossfades dry/wet and truncates to input length.
pub(super) fn convolve(input: &[f32], spec: ConvolveSpec, sr: u32, seed: u64) -> Signal {
    let mix = spec.mix.clamp(0.0, 1.0);
    if mix == 0.0 {
        // Transparent passthrough, bit-identical by construction.
        return input.to_vec();
    }
    let ir = synth_ir(spec.decay, spec.size, spec.predelay, spec.damp, sr, seed);

    let wet = crate::det::convolve(input, &ir);
    input
        .iter()
        .zip(&wet)
        .map(|(&dry, &w)| dry * (1.0 - mix) + w * mix)
        .collect()
}

/// The `granular` node's parameters, bundled so the processor function stays
/// under the arity cap.
#[derive(Clone, Copy)]
pub(super) struct GranularSpec {
    /// Grain length in milliseconds.
    pub grain_ms: f32,
    /// Grains per second.
    pub density: f32,
    /// Grain playback ratio.
    pub pitch: f32,
    /// Randomization depth, 0..1.
    pub spread: f32,
    /// Dry/wet mix, 0..1.
    pub mix: f32,
}

/// One grain of a [`granular`] schedule: where it lands in the output, where
/// it starts reading the input, and its playback ratio.
struct Grain {
    out: usize,
    src: usize,
    ratio: f32,
}

/// Granular texture: the input is chopped into overlapping Hann-windowed
/// grains at `density` grains/sec; each grain replays the input from its
/// (jittered) source position at `pitch` (playback ratio), with `spread`
/// scaling the onset/source jitter and a per-grain detune of up to ±half an
/// octave. The whole schedule is drawn up front from the node's structural
/// stream in a fixed per-grain draw order (onset jitter → detune → source
/// jitter), so the texture is deterministic and edit-stable. Grains are
/// summed and normalized by the window-overlap envelope, so loudness tracks
/// the dry signal, then crossfaded per `mix`. Offline only — grains read the
/// whole input out of order, so the streaming renderer refuses this node.
pub(super) fn granular(input: &[f32], spec: GranularSpec, sr: u32, seed: u64) -> Signal {
    let mix = spec.mix.clamp(0.0, 1.0);
    if mix == 0.0 {
        // Transparent passthrough, bit-identical by construction.
        return input.to_vec();
    }
    let srf = sr as f32;
    let n = input.len();
    // validate() bounds every knob; the clamps guard direct renders.
    let g = ((spec.grain_ms.clamp(5.0, 500.0) / 1000.0) * srf).max(1.0) as usize;
    let hop = (srf / spec.density.clamp(0.1, 200.0)).max(1.0);
    let pitch = spec.pitch.clamp(0.25, 4.0);
    let spread = spec.spread.clamp(0.0, 1.0);
    // Hann window (sin² — the analysis module's window).
    let win: Vec<f32> = (0..g)
        .map(|i| {
            let x = PI * i as f32 / (g as f32 - 1.0).max(1.0);
            let s = dsp::sin(x);
            s * s
        })
        .collect();
    let count = (n as f32 / hop).ceil() as usize + 1;
    let mut rng = Rng::new(seed);
    let grains: Vec<Grain> = (0..count)
        .map(|k| {
            let onset = k as f32 * hop;
            // Fixed draw order per grain, independent of everything else.
            let out_j = rng.bi() * spread * hop;
            let detune = rng.bi() * spread * 0.5; // octaves
            let src_j = rng.bi() * spread * hop;
            Grain {
                out: (onset + out_j).max(0.0) as usize,
                src: (onset + src_j).max(0.0) as usize,
                ratio: pitch * dsp::powf(2.0, detune),
            }
        })
        .collect();
    let mut wet = vec![0.0f32; n];
    let mut wsum = vec![0.0f32; n];
    for gr in &grains {
        for (j, &w) in win.iter().enumerate() {
            let o = gr.out + j;
            if o >= n {
                break;
            }
            // Fractional-delay read (linear interpolation) of the source.
            let read = gr.src as f32 + j as f32 * gr.ratio;
            let i0 = read as usize;
            if i0 + 1 >= n {
                break;
            }
            let frac = read - i0 as f32;
            wet[o] += (input[i0] * (1.0 - frac) + input[i0 + 1] * frac) * w;
            wsum[o] += w;
        }
    }
    input
        .iter()
        .enumerate()
        .map(|(i, &dry)| {
            // Normalize by the overlap envelope; the 1.0 floor fades the
            // sparsely covered doc edges instead of boosting them.
            let w = wet[i] / wsum[i].max(1.0);
            dry * (1.0 - mix) + w * mix
        })
        .collect()
}
