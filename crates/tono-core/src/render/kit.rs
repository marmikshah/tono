//! Drum-kit voices: General-MIDI-mapped synthesized drum hits in the four
//! kit styles, plus the shared cowbell/metallic partial banks.

use super::Signal;
use crate::dsl::KitStyle;
use crate::dsp::{self, Rng};
use std::f32::consts::TAU;

/// One sample of cowbell at fundamental `f`: two clashing partials (the
/// classic ~1.56 ratio of an 808 cowbell), saturated square-ish, with a fast
/// knock decay.
pub(super) fn cowbell_sample(f: f32, t: f32) -> f32 {
    let a = dsp::tanh(2.5 * dsp::sin(TAU * f * t));
    let b = dsp::tanh(2.5 * dsp::sin(TAU * f * 1.565 * t));
    0.5 * (a + b) * dsp::exp(-t / 0.09)
}

/// One General-MIDI-mapped drum hit — the note's onset pitch picks the voice,
/// `style` the kit. `Classic` is the original kit, byte-frozen; the other
/// styles are alternate synthesized voicings.
pub(super) fn kit_drum(f: &[f32], sr: u32, rng: &mut Rng, style: KitStyle) -> Signal {
    match style {
        KitStyle::Classic => kit_drum_classic(f, sr, rng),
        KitStyle::Acoustic => kit_drum_acoustic(f, sr, rng),
        KitStyle::Electronic => kit_drum_electronic(f, sr, rng),
        KitStyle::Eight08 => kit_drum_808(f, sr, rng),
    }
}

fn kit_drum_classic(f: &[f32], sr: u32, rng: &mut Rng) -> Signal {
    let srf = sr as f32;
    let n = f.len();
    // Recover the MIDI number from the onset frequency (pitch is wire-encoded
    // as Hz; "midi:36" round-trips exactly).
    let midi = crate::dsp::hz_to_midi(f[0].max(8.0)).round() as i32;
    let mut out = Vec::with_capacity(n);

    // One-pole highpass state for cymbal/snare noise.
    let (mut lp, hp_a) = (0.0f32, 1.0 - dsp::exp(-TAU * 5_500.0 / srf));
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
                let fk = 45.0 + 105.0 * dsp::exp(-t / 0.04);
                phase += fk / srf;
                phase -= phase.floor();
                let click = if t < 0.002 { rng.bi() * 0.4 } else { 0.0 };
                dsp::sin(TAU * phase) * dsp::exp(-t / 0.13) + click
            }
            // Snare / rimshot / clap: tone crack + noise body.
            38 | 40 => {
                let tone = dsp::sin(TAU * 190.0 * t) * 0.4 * dsp::exp(-t / 0.06);
                tone + rng.bi() * 0.8 * dsp::exp(-t / 0.11)
            }
            37 => {
                dsp::sin(TAU * 800.0 * t) * 0.3 * dsp::exp(-t / 0.03)
                    + rng.bi() * dsp::exp(-t / 0.025)
            }
            39 => rng.bi() * dsp::exp(-t / 0.09),
            // Hats: highpassed noise, closed dies fast, open rings.
            42 | 44 => hp(rng.bi(), &mut lp) * dsp::exp(-t / 0.035),
            46 => hp(rng.bi(), &mut lp) * dsp::exp(-t / 0.22),
            // Toms: pitched thumps falling with the GM map.
            41 | 43 | 45 | 47 | 48 | 50 => {
                let base = 80.0 + 24.0 * (midi - 41) as f32;
                let ft = base * (1.0 - 0.15 * (t / 0.2).min(1.0));
                phase += ft / srf;
                phase -= phase.floor();
                dsp::sin(TAU * phase) * dsp::exp(-t / 0.18) + rng.bi() * 0.1 * dsp::exp(-t / 0.03)
            }
            // Cowbell (more cowbell).
            56 => cowbell_sample(540.0, t),
            // Crash / ride.
            49 | 55 | 57 => hp(rng.bi(), &mut lp) * dsp::exp(-t / 0.7),
            51 | 53 | 59 => {
                hp(rng.bi(), &mut lp) * 0.5 * dsp::exp(-t / 0.45)
                    + dsp::sin(TAU * 5_200.0 * t) * 0.25 * dsp::exp(-t / 0.25)
            }
            // Anything unmapped: a generic percussive hit.
            _ => rng.bi() * dsp::exp(-t / 0.08),
        };
        out.push(s);
    }
    out
}

// The alternate kit styles keep the classic GM note map and the per-sample /
// in-order-`rng.bi()` discipline so they stream byte-identically.

/// Deeper, roomier acoustic kit: pitch-dropping kick with a beater knock, a
/// tuned-head snare with crack and buzz, ringing toms, shimmering cymbals.
fn kit_drum_acoustic(f: &[f32], sr: u32, rng: &mut Rng) -> Signal {
    let srf = sr as f32;
    let n = f.len();
    let midi = crate::dsp::hz_to_midi(f[0].max(8.0)).round() as i32;
    let a = |fc: f32| 1.0 - dsp::exp(-TAU * fc / srf);
    let (a3000, a3500, a4000, a2500, a400, a900) = (
        a(3000.0),
        a(3500.0),
        a(4000.0),
        a(2500.0),
        a(400.0),
        a(900.0),
    );
    let (a11000, a8000, a6500, a2000, a12000, a7000) = (
        a(11000.0),
        a(8000.0),
        a(6500.0),
        a(2000.0),
        a(12000.0),
        a(7000.0),
    );
    let (mut lpa, mut lpb, mut hpa) = (0.0f32, 0.0f32, 0.0f32);
    let (mut phase, mut phase2) = (0.0f32, 0.0f32);
    let mut out = Vec::with_capacity(n);
    for i in 0..n {
        let t = i as f32 / srf;
        let s = match midi {
            35 | 36 => {
                let fk = 48.0 + 140.0 * dsp::exp(-t / 0.028);
                phase += fk / srf;
                phase -= phase.floor();
                let body = dsp::sin(TAU * phase) * dsp::exp(-t / 0.16);
                let click = if t < 0.018 {
                    let w = rng.bi();
                    lpa += a3000 * (w - lpa);
                    let tick = dsp::sin(TAU * 2600.0 * t) * dsp::exp(-t / 0.004);
                    (w - lpa) * 0.50 * dsp::exp(-t / 0.007) + 0.28 * tick
                } else {
                    0.0
                };
                0.90 * body + click
            }
            38 | 40 => {
                let w = rng.bi();
                let m1 = dsp::sin(TAU * 185.0 * t) * 0.48 * dsp::exp(-t / 0.10);
                let m2 = dsp::sin(TAU * 330.0 * t) * 0.26 * dsp::exp(-t / 0.07);
                lpa += a3500 * (w - lpa);
                let crack = (w - lpa) * 0.55 * dsp::exp(-t / 0.035);
                lpb += a2500 * (w - lpb);
                hpa += a400 * (lpb - hpa);
                let buzz = (lpb - hpa) * 0.40 * dsp::exp(-t / 0.13);
                m1 + m2 + crack + buzz
            }
            37 => {
                let w = rng.bi();
                lpa += a4000 * (w - lpa);
                let snap = (w - lpa) * 0.50 * dsp::exp(-t / 0.012);
                let ring = dsp::sin(TAU * 1700.0 * t) * 0.35 * dsp::exp(-t / 0.008);
                let knock = dsp::sin(TAU * 420.0 * t) * 0.30 * dsp::exp(-t / 0.03);
                snap + ring + knock
            }
            39 => {
                let w = rng.bi();
                lpa += a2500 * (w - lpa);
                hpa += a900 * (lpa - hpa);
                let band = lpa - hpa;
                let burst = |d: f32| {
                    if t >= d {
                        dsp::exp(-(t - d) / 0.009)
                    } else {
                        0.0
                    }
                };
                let bursts = (burst(0.0) + burst(0.009) + burst(0.018) + burst(0.027)).min(1.0);
                let tail = 0.35 * dsp::exp(-t / 0.10);
                band * (0.90 * bursts + tail)
            }
            42 | 44 => {
                let w = rng.bi();
                lpa += a11000 * (w - lpa);
                hpa += a8000 * (lpa - hpa);
                let shimmer = lpa - hpa;
                lpb += a6500 * (w - lpb);
                (0.60 * (w - lpb) + 0.55 * shimmer) * dsp::exp(-t / 0.032)
            }
            46 => {
                let w = rng.bi();
                lpa += a11000 * (w - lpa);
                hpa += a8000 * (lpa - hpa);
                let shimmer = lpa - hpa;
                lpb += a6500 * (w - lpb);
                let env = 0.85 * dsp::exp(-t / 0.32) + 0.15 * dsp::exp(-t / 0.08);
                (0.55 * (w - lpb) + 0.60 * shimmer) * env
            }
            41 | 43 | 45 | 47 | 48 | 50 => {
                let base = 80.0 + 24.0 * (midi - 41) as f32;
                let ft = base * (1.0 - 0.12 * (t / 0.25).min(1.0));
                phase += ft / srf;
                phase -= phase.floor();
                phase2 += 1.59 * ft / srf;
                phase2 -= phase2.floor();
                let fund = dsp::sin(TAU * phase) * dsp::exp(-t / 0.35);
                let mode = dsp::sin(TAU * phase2) * 0.30 * dsp::exp(-t / 0.14);
                let w = rng.bi();
                lpa += a2000 * (w - lpa);
                let stick = (w - lpa) * 0.18 * dsp::exp(-t / 0.008);
                0.85 * fund + mode + stick
            }
            56 => cowbell_sample(540.0, t),
            49 | 55 | 57 => {
                let w = rng.bi();
                lpa += a2500 * (w - lpa);
                let wash = (w - lpa) * 0.60 * dsp::exp(-t / 0.90);
                lpb += a12000 * (w - lpb);
                hpa += a7000 * (lpb - hpa);
                let lfo = 0.6 + 0.4 * dsp::sin(TAU * 6.0 * t);
                let shine = (lpb - hpa) * lfo * 0.50 * dsp::exp(-t / 0.70);
                let clash = (dsp::sin(TAU * 3300.0 * t)
                    + dsp::sin(TAU * 5240.0 * t)
                    + dsp::sin(TAU * 8130.0 * t))
                    * 0.06
                    * dsp::exp(-t / 0.22);
                wash + shine + clash
            }
            51 | 53 | 59 => {
                let w = rng.bi();
                lpa += a3000 * (w - lpa);
                let wash = (w - lpa) * 0.45 * dsp::exp(-t / 0.55);
                lpb += a12000 * (w - lpb);
                hpa += a8000 * (lpb - hpa);
                let shine = (lpb - hpa) * 0.40 * dsp::exp(-t / 0.40);
                let ping = (dsp::sin(TAU * 2100.0 * t) * 0.5
                    + dsp::sin(TAU * 3170.0 * t) * 0.3
                    + dsp::sin(TAU * 4200.0 * t) * 0.2)
                    * dsp::exp(-t / 0.30);
                0.50 * ping + wash + shine
            }
            _ => {
                let w = rng.bi();
                lpa += a4000 * (w - lpa);
                (0.5 * w + 0.5 * lpa) * dsp::exp(-t / 0.08)
            }
        };
        out.push(s);
    }
    out
}

/// Clean synthesized electronic kit: driven synth kick, gated snare, zappy toms,
/// glassy super-bright hats and cymbals.
fn kit_drum_electronic(f: &[f32], sr: u32, rng: &mut Rng) -> Signal {
    let srf = sr as f32;
    let n = f.len();
    let midi = crate::dsp::hz_to_midi(f[0].max(8.0)).round() as i32;
    let a5500 = 1.0 - dsp::exp(-TAU * 5500.0 / srf);
    let a9000 = 1.0 - dsp::exp(-TAU * 9000.0 / srf);
    let (mut lp, mut lp2, mut phase) = (0.0f32, 0.0f32, 0.0f32);
    let mut out = Vec::with_capacity(n);
    for i in 0..n {
        let t = i as f32 / srf;
        let s = match midi {
            35 | 36 => {
                let fk = 55.0 + 145.0 * dsp::exp(-t / 0.025);
                phase += fk / srf;
                phase -= phase.floor();
                let body = dsp::tanh(1.3 * dsp::sin(TAU * phase)) * 0.85 * dsp::exp(-t / 0.11);
                let click = if t < 0.003 { rng.bi() * 0.3 } else { 0.0 };
                body + click
            }
            38 | 40 => {
                let tone = (dsp::sin(TAU * 185.0 * t) * 0.45 + dsp::sin(TAU * 330.0 * t) * 0.22)
                    * dsp::exp(-t / 0.055);
                let gate = if t < 0.13 {
                    1.0
                } else {
                    dsp::exp(-(t - 0.13) / 0.006)
                };
                let w = rng.bi();
                lp += a5500 * (w - lp);
                tone + (w - lp) * 0.7 * dsp::exp(-t / 0.16) * gate
            }
            37 => {
                dsp::sin(TAU * 1700.0 * t) * 0.5 * dsp::exp(-t / 0.012)
                    + dsp::sin(TAU * 420.0 * t) * 0.3 * dsp::exp(-t / 0.02)
                    + rng.bi() * 0.3 * dsp::exp(-t / 0.006)
            }
            39 => {
                let ev = if t < 0.03 {
                    dsp::exp(-((t % 0.01) / 0.003))
                } else {
                    dsp::exp(-(t - 0.03) / 0.10)
                };
                let w = rng.bi();
                lp += a5500 * (w - lp);
                (0.5 * (w - lp) + 0.5 * w) * ev * 0.9
            }
            42 | 44 => {
                let w = rng.bi();
                lp2 += a9000 * (w - lp2);
                (w - lp2) * 1.5 * dsp::exp(-t / 0.02)
            }
            46 => {
                let w = rng.bi();
                lp2 += a9000 * (w - lp2);
                (w - lp2) * 1.4 * dsp::exp(-t / 0.18)
                    + dsp::sin(TAU * 9000.0 * t)
                        * dsp::sin(TAU * 11500.0 * t)
                        * 0.1
                        * dsp::exp(-t / 0.14)
            }
            41 | 43 | 45 | 47 | 48 | 50 => {
                let base = 90.0 + 26.0 * (midi - 41) as f32;
                let ft = base * (1.0 + 1.5 * dsp::exp(-t / 0.05));
                phase += ft / srf;
                phase -= phase.floor();
                dsp::tanh(1.2 * dsp::sin(TAU * phase)) * dsp::exp(-t / 0.16)
                    + rng.bi() * 0.08 * dsp::exp(-t / 0.02)
            }
            56 => cowbell_sample(555.0, t),
            49 | 55 | 57 => {
                let w = rng.bi();
                lp2 += a9000 * (w - lp2);
                (w - lp2) * 1.4 * dsp::exp(-t / 0.6)
                    + dsp::sin(TAU * 8000.0 * t)
                        * dsp::sin(TAU * 11000.0 * t)
                        * 0.12
                        * dsp::exp(-t / 0.5)
            }
            51 | 53 | 59 => {
                let w = rng.bi();
                lp2 += a9000 * (w - lp2);
                dsp::sin(TAU * 5800.0 * t) * 0.35 * dsp::exp(-t / 0.35)
                    + dsp::sin(TAU * 8700.0 * t) * 0.15 * dsp::exp(-t / 0.3)
                    + (w - lp2) * 0.4 * dsp::exp(-t / 0.5)
            }
            _ => {
                let w = rng.bi();
                lp2 += a9000 * (w - lp2);
                (w - lp2) * 1.6 * dsp::exp(-t / 0.06)
            }
        };
        out.push(s);
    }
    out
}

/// The Roland TR-808 hi-hat/cymbal oscillator bank: six hard-square partials.
fn metal_808(t: f32) -> f32 {
    const FS: [f32; 6] = [205.3, 304.4, 369.6, 522.7, 540.0, 800.0];
    let mut s = 0.0;
    for &fr in &FS {
        s += dsp::sin(TAU * fr * t).signum();
    }
    s / 6.0
}

/// 808-style kit: a long booming sub-sine kick, papery clap, snappy snare, tick
/// clave, ringy square cowbell, buzzy metallic hats/cymbals.
fn kit_drum_808(f: &[f32], sr: u32, rng: &mut Rng) -> Signal {
    let srf = sr as f32;
    let n = f.len();
    let midi = crate::dsp::hz_to_midi(f[0].max(8.0)).round() as i32;
    let a6000 = 1.0 - dsp::exp(-TAU * 6000.0 / srf);
    let clo_a = 1.0 - dsp::exp(-TAU * 2200.0 / srf);
    let chi_a = 1.0 - dsp::exp(-TAU * 700.0 / srf);
    let (mut hlp, mut clp, mut chp, mut phase) = (0.0f32, 0.0f32, 0.0f32, 0.0f32);
    let mut out = Vec::with_capacity(n);
    for i in 0..n {
        let t = i as f32 / srf;
        let s = match midi {
            35 | 36 => {
                let fk = 52.0 + 68.0 * dsp::exp(-t / 0.025);
                phase += fk / srf;
                phase -= phase.floor();
                let body = dsp::sin(TAU * phase) * dsp::exp(-t / 0.60);
                let click = if t < 0.004 {
                    (rng.bi() * 0.5 + dsp::sin(TAU * 1600.0 * t) * 0.5) * dsp::exp(-t / 0.0015)
                } else {
                    0.0
                };
                body + click * 0.5
            }
            38 | 40 => {
                let tone = (dsp::sin(TAU * 175.0 * t) + dsp::sin(TAU * 330.0 * t))
                    * 0.32
                    * dsp::exp(-t / 0.10);
                let w = rng.bi();
                hlp += a6000 * (w - hlp);
                tone + (w - hlp) * 0.7 * dsp::exp(-t / 0.07)
            }
            37 => {
                let tick = dsp::sin(TAU * 1700.0 * t) * 0.7 * dsp::exp(-t / 0.006);
                let snap = if t < 0.003 {
                    rng.bi() * 0.3 * dsp::exp(-t / 0.001)
                } else {
                    0.0
                };
                tick + snap
            }
            39 => {
                let ph = (t % 0.010) / 0.010;
                let burst = if t < 0.030 { dsp::exp(-ph / 0.22) } else { 0.0 };
                let env = burst + 0.55 * dsp::exp(-t / 0.12);
                let w = rng.bi();
                clp += clo_a * (w - clp);
                chp += chi_a * (clp - chp);
                (clp - chp) * env * 1.1
            }
            42 | 44 => {
                let m = metal_808(t);
                hlp += a6000 * (m - hlp);
                (m - hlp) * 0.55 * dsp::exp(-t / 0.05)
            }
            46 => {
                let m = metal_808(t);
                hlp += a6000 * (m - hlp);
                (m - hlp) * 0.55 * dsp::exp(-t / 0.35)
            }
            41 | 43 | 45 | 47 | 48 | 50 => {
                let base = 90.0 + 26.0 * (midi - 41) as f32;
                let ft = base * (1.0 + 0.6 * dsp::exp(-t / 0.02));
                phase += ft / srf;
                phase -= phase.floor();
                let dec = 0.32 - 0.025 * (midi - 41) as f32;
                dsp::sin(TAU * phase) * dsp::exp(-t / dec)
            }
            56 => {
                let a = dsp::sin(TAU * 540.0 * t).signum();
                let b = dsp::sin(TAU * 845.0 * t).signum();
                0.4 * (a + b) * dsp::exp(-t / 0.20)
            }
            49 | 55 | 57 => {
                let w = rng.bi();
                let mix = metal_808(t) * 0.6 + w * 0.5;
                hlp += a6000 * (mix - hlp);
                (mix - hlp) * 0.7 * dsp::exp(-t / 0.90)
            }
            51 | 53 | 59 => {
                let w = rng.bi();
                let mix = metal_808(t) * 0.5 + w * 0.3;
                hlp += a6000 * (mix - hlp);
                (mix - hlp) * 0.6 * dsp::exp(-t / 0.50)
                    + dsp::sin(TAU * 5200.0 * t) * 0.20 * dsp::exp(-t / 0.30)
            }
            _ => rng.bi() * dsp::exp(-t / 0.08),
        };
        out.push(s);
    }
    out
}
