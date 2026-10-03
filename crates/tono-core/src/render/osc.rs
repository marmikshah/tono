//! Oscillator and noise sources: band-limited (PolyBLEP) waveforms, unison
//! super-oscillators, FM, coloured noise, dust, and the impact exciter.

use super::{Signal, eval_value};
use crate::dsl::{NoiseColor, Shape, SuperWave, Value, WavetableKind};
use crate::dsp::{self, Rng};
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
/// the two frames adjacent to `position` (clamped to 0..1). THE per-sample
/// definition — the offline loop and the streaming `Src` both call this, so
/// the two paths share an identical op order.
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

/// Morphing wavetable oscillator: `position` sweeps the frame crossfade,
/// `freq` drives the table phase — both evaluated per-sample like every
/// other oscillator, so a modulated morph renders identically offline and
/// streamed.
pub(super) fn wavetable_signal(
    wave: WavetableKind,
    freq: &Value,
    position: &Value,
    n: usize,
    sr: u32,
) -> Signal {
    let frames = wavetable_frames(wave);
    let f = eval_value(freq, n, sr);
    let p = eval_value(position, n, sr);
    let srf = sr as f32;
    let mut phase = 0.0f32;
    let mut out = Vec::with_capacity(n);
    for i in 0..n {
        out.push(wavetable_lookup(&frames, p[i], phase));
        phase += f[i].max(0.0) / srf;
        phase -= phase.floor();
    }
    out
}

/// Sparse stochastic impulses — a Poisson click train smoothed by a one-pole
/// decay so overlapping grains sum. `density` events/sec each fire with random
/// ± amplitude; `decay` sets the per-grain ring (0 = bare impulses). Draws from
/// the render stream (like [`noise_signal`]).
pub(super) fn dust_signal(density: f32, decay: f32, n: usize, sr: u32, rng: &mut Rng) -> Signal {
    let srf = sr as f32;
    let p = (density / srf).clamp(0.0, 1.0); // event probability per sample
    let g = if decay > 0.0 {
        dsp::exp(-1.0 / (decay * srf))
    } else {
        0.0
    };
    let mut y = 0.0f32;
    (0..n)
        .map(|_| {
            let imp = if rng.unit() < p { rng.bi() } else { 0.0 };
            y = imp + g * y;
            y
        })
        .collect()
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

/// Impact exciter: a single raised-cosine (Hann-lobe) force pulse at the start
/// of the buffer, the rest silence. `hardness` sets the contact time — a hard
/// strike is a brief, wide-band click (lights up high modes); a soft strike is
/// a longer, duller bump. The pulse is normalised to UNIT AREA (× `velocity`),
/// so `velocity` sets the total impulse delivered and `hardness` only shapes
/// its spectrum — a downstream [`Node::Modal`] bank then rings to a level set
/// by its modes' `gain`, independent of the strike width.
pub(super) fn impact_signal(hardness: f32, velocity: f32, n: usize, sr: u32) -> Signal {
    let h = hardness.clamp(0.0, 1.0);
    let v = velocity.clamp(0.0, 1.0);
    // Contact time: soft ≈ 8 ms, hard ≈ 0.3 ms.
    let width_s = 0.008 * (1.0 - h) + 0.0003 * h;
    let w = ((width_s * sr as f32).round() as usize).max(1);
    // A Hann lobe sums to ≈ w/2; normalise so the whole pulse has area `v`.
    let norm = v / (0.5 * w as f32);
    let mut out = vec![0.0f32; n];
    for (i, o) in out.iter_mut().enumerate().take(w.min(n)) {
        let phase = (i as f32 + 0.5) / w as f32;
        *o = norm * 0.5 * (1.0 - dsp::cos(TAU * phase));
    }
    out
}

/// Drive a phase accumulator at a (possibly modulated) frequency and map each
/// phase to a sample via `wave`.
pub(super) fn osc_signal(freq: &Value, n: usize, sr: u32, wave: impl Fn(f32) -> f32) -> Signal {
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
pub(super) fn square_signal(freq: &Value, duty: &Value, n: usize, sr: u32) -> Signal {
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
pub(super) fn saw_signal(freq: &Value, n: usize, sr: u32) -> Signal {
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
pub(super) fn tri_signal(freq: &Value, n: usize, sr: u32) -> Signal {
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
pub(super) fn super_signal(
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
        let ratio = dsp::powf(2.0, cents / 1200.0);
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
pub(super) fn noise_signal(color: NoiseColor, n: usize, rng: &mut Rng) -> Signal {
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
pub(super) fn fm_signal(freq: &Value, ratio: f32, index: &Value, n: usize, sr: u32) -> Signal {
    let f = eval_value(freq, n, sr);
    let idx = eval_value(index, n, sr);
    let srf = sr as f32;
    let (mut cph, mut mph) = (0.0f32, 0.0f32);
    let mut out = Vec::with_capacity(n);
    for i in 0..n {
        let m = idx[i] * dsp::sin(TAU * mph);
        out.push(dsp::sin(TAU * cph + m));
        let fi = f[i].max(0.0);
        cph += fi / srf;
        cph -= cph.floor();
        mph += (fi * ratio) / srf;
        mph -= mph.floor();
    }
    out
}
