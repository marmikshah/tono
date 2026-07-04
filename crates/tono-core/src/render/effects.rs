//! Filters and effect processors: the biquad family, reverb, the waveshaper
//! (with ADAA), the modal resonator bank, modulation effects, and dynamics.

use super::{Signal, eval_value};
use crate::dsl::{DriveShape, Mode, Value};
use std::f32::consts::{LN_2, TAU};

#[derive(Clone, Copy)]
pub(super) enum FilterKind {
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
pub(super) fn biquad(input: &[f32], cutoff: &Value, q: f32, sr: u32, kind: FilterKind) -> Signal {
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
pub(super) fn reverb(input: &[f32], room: f32, mix: f32, sr: u32, spread: usize) -> Signal {
    let scale = sr as f32 / 44_100.0;
    let comb_tunings = crate::dsp::FREEVERB_COMB_TUNINGS.map(|t| t + spread);
    let allpass_tunings = crate::dsp::FREEVERB_ALLPASS_TUNINGS.map(|t| t + spread);
    let feedback = 0.7 + 0.28 * room.clamp(0.0, 1.0);
    let damp = crate::dsp::FREEVERB_DAMP;

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
pub(crate) fn drive_curve(x: f32, shape: DriveShape) -> f32 {
    match shape {
        DriveShape::Tanh => x.tanh(),
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

/// Antiderivative F(x) of each waveshaper, used by [`drive_adaa`]. F'(x) =
/// `drive_curve(x, shape)`. The additive constant is irrelevant — ADAA only
/// ever uses differences `F(x1) − F(x0)`.
pub(crate) fn drive_antideriv(x: f32, shape: DriveShape) -> f32 {
    match shape {
        // ∫ tanh = ln(cosh x). Computed as |x| + ln(1+e^{−2|x|}) − ln 2 so it
        // never overflows for large |x| (cosh would).
        DriveShape::Tanh => {
            let a = x.abs();
            a + (-2.0 * a).exp().ln_1p() - LN_2
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
    let srf = sr as f32;
    let nyq = srf * 0.5;
    let mix = mix.clamp(0.0, 1.0);
    let mut wet = vec![0.0f32; input.len()];
    for m in modes {
        let f0 = m.freq.clamp(1.0, nyq - 1.0);
        let decay = m.decay.max(1e-3);
        let w0 = TAU * f0 / srf;
        let (sin0, cos0) = (w0.sin(), w0.cos());
        // r so the ring reaches −60 dB (×0.001) after `decay` seconds.
        let r = (-6.907_755 / (decay * srf)).exp();
        let a1 = 2.0 * r * cos0;
        let a2 = -r * r;
        let b0 = m.gain * sin0; // impulse-response peak ≈ gain
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
pub(super) fn compress(
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
