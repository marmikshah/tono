//! Effect kernels shared with the evaluator: the biquad coefficient table
//! and the waveshaper curves with their antiderivatives (ADAA).

use crate::dsl::DriveShape;
use crate::dsp;
use std::f32::consts::{LN_2, TAU};

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
