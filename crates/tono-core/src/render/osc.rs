//! Oscillator kernels shared by the evaluator and the seq synthesis: the
//! basic waveforms, the PolyBLEP residual, and the band-limited wavetables.

use crate::dsl::{Shape, WavetableKind};
use crate::dsp;
use std::f32::consts::TAU;

/// Wavetable frame length (one cycle) and the partial cap every sub-wave is
/// band-limited to at GENERATION time. 32 partials is alias-free up to
/// ~0.45·sr/32 ≈ 620 Hz at 44.1 kHz; above that, bright positions fold —
/// the documented trade-off of a fixed-table design (darker sub-waves use
/// fewer partials and stay clean higher).
pub(crate) const WT_LEN: usize = 2048;
pub(crate) const WT_MAX_PARTIAL: u32 = 32;

/// Build one single-cycle table frame from `(partial, amplitude)` pairs,
/// peak-normalized through the deterministic sine kernel.
fn additive_frame(partials: &[(u32, f32)]) -> Vec<f32> {
    let mut frame = vec![0.0f32; WT_LEN];
    for (i, s) in frame.iter_mut().enumerate() {
        let phase = i as f32 / WT_LEN as f32;
        let mut acc = 0.0f32;
        for &(h, a) in partials {
            acc += a * dsp::sin(TAU * h as f32 * phase);
        }
        *s = acc;
    }
    let peak = frame.iter().fold(0.0f32, |m, &x| m.max(x.abs()));
    if peak > 0.0 {
        for s in frame.iter_mut() {
            *s /= peak;
        }
    }
    frame
}

/// Saw stack: every partial 1..=cap at amplitude 1/k.
fn saw_partials(cap: u32) -> Vec<(u32, f32)> {
    (1..=cap.min(WT_MAX_PARTIAL))
        .map(|k| (k, 1.0 / k as f32))
        .collect()
}

/// Square stack: odd partials at 1/k.
fn square_partials(cap: u32) -> Vec<(u32, f32)> {
    (1..=cap.min(WT_MAX_PARTIAL))
        .step_by(2)
        .map(|k| (k, 1.0 / k as f32))
        .collect()
}

/// Triangle stack: odd partials at ±1/k² (alternating sign).
fn tri_partials(cap: u32) -> Vec<(u32, f32)> {
    (1..=cap.min(WT_MAX_PARTIAL))
        .step_by(2)
        .enumerate()
        .map(|(i, k)| {
            let sign = if i % 2 == 0 { 1.0 } else { -1.0 };
            (k, sign / (k as f32 * k as f32))
        })
        .collect()
}

/// Build the shared, band-limited wavetable frames with deterministic math.
pub(crate) fn wavetable_frames(kind: WavetableKind) -> Vec<Vec<f32>> {
    match kind {
        WavetableKind::Basic => vec![
            additive_frame(&[(1, 1.0)]),
            additive_frame(&tri_partials(16)),
            additive_frame(&square_partials(32)),
            additive_frame(&saw_partials(32)),
        ],
        WavetableKind::Harmonics => [1, 2, 4, 8, 16, 32]
            .iter()
            .map(|&cap| additive_frame(&saw_partials(cap)))
            .collect(),
        // Formant centres against a ~110 Hz reference: a 660/1100, e 550/1870,
        // i 330/2310, o 550/880, u 330/880 — snapped to integer partials so
        // every frame is perfectly periodic.
        WavetableKind::Formant => [
            &[(1, 1.0), (6, 0.6), (10, 0.5)][..],
            &[(1, 1.0), (5, 0.55), (17, 0.45)][..],
            &[(1, 1.0), (3, 0.4), (21, 0.55)][..],
            &[(1, 1.0), (5, 0.6), (8, 0.45)][..],
            &[(1, 1.0), (3, 0.5), (8, 0.4)][..],
        ]
        .iter()
        .map(|p| additive_frame(p))
        .collect(),
        WavetableKind::Metallic => [
            &[(1, 1.0), (5, 0.5), (10, 0.4)][..],
            &[(1, 1.0), (3, 0.5), (7, 0.4), (13, 0.3)][..],
            &[(2, 1.0), (5, 0.6), (9, 0.45), (16, 0.35)][..],
            &[(1, 1.0), (4, 0.5), (11, 0.45), (19, 0.35), (26, 0.3)][..],
        ]
        .iter()
        .map(|p| additive_frame(p))
        .collect(),
    }
}

/// One wavetable sample: linear table lookup at `phase` (0..1), crossfading
/// the two frames adjacent to `position` (clamped to 0..1).
pub(crate) fn wavetable_lookup(frames: &[Vec<f32>], position: f32, phase: f32) -> f32 {
    let read = |frame: &[f32]| {
        let u = phase * WT_LEN as f32;
        let i = u as usize;
        let frac = u - i as f32;
        let a = frame[i];
        a + frac * (frame[(i + 1) % WT_LEN] - a)
    };
    let x = position.clamp(0.0, 1.0) * (frames.len() - 1) as f32;
    let lo = x as usize;
    let frac = x - lo as f32;
    let s0 = read(&frames[lo]);
    if frac == 0.0 {
        return s0;
    }
    let s1 = read(&frames[(lo + 1).min(frames.len() - 1)]);
    s0 + frac * (s1 - s0)
}

/// PolyBLEP residual for band-limited oscillators: corrects the discontinuity
/// at a phase edge to suppress aliasing. `t` is the phase (0..1), `dt` the
/// per-sample phase increment.
pub(crate) fn poly_blep(mut t: f32, dt: f32) -> f32 {
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
pub(crate) fn osc(shape: Shape, phase: f32) -> f32 {
    match shape {
        Shape::Sine => dsp::sin(TAU * phase),
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
