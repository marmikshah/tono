//! Effect kernels shared with the evaluator: the biquad coefficient table,
//! the waveshaper curves and their antiderivatives (ADAA), and the two
//! whole-buffer effects — synthesized-IR convolution and the granular
//! texture — that only the bounce can run.

use super::Signal;
use crate::dsl::DriveShape;
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

/// The `convolve` node's parameters.
#[derive(Clone, Copy)]
pub(crate) struct ConvolveSpec {
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
pub(crate) fn convolve(input: &[f32], spec: ConvolveSpec, sr: u32, seed: u64) -> Signal {
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

/// The `granular` node's parameters.
#[derive(Clone, Copy)]
pub(crate) struct GranularSpec {
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
pub(crate) fn granular(input: &[f32], spec: GranularSpec, sr: u32, seed: u64) -> Signal {
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
