//! Processors: the stateful block evaluators of every transforming node
//! (filters, delays, dynamics, waveshaping), and the graph walk that builds
//! them.

use std::f32::consts::TAU;

use super::Ctx;
use super::value::Val;
use crate::dsl::{DriveShape, Node};
use crate::dsp;
use crate::render::{FilterKind, biquad_coeffs, drive_antideriv, drive_curve};

/// One resonator of a [`Proc::Modal`] bank: constant LTI coeffs + 2-pole state.
pub(super) struct ModalMode {
    a1: f32,
    a2: f32,
    b0: f32,
    y1: f32,
    y2: f32,
}

/// A processor holding its state between blocks. `process` transforms the
/// samples at `t0..t0 + buf.len()` in place.
pub(super) enum Proc {
    Gain(Val),
    Biquad {
        kind: FilterKind,
        /// The authored cutoff, evaluated per sample; the coefficients are
        /// recomputed only when it moves, so a constant cutoff pays for them
        /// once and a swept one is bit-identical to the per-sample form.
        cutoff: Val,
        /// The live cutoff multiplier (`set_cutoff`), 1.0 = as built.
        scale: f32,
        q: f32,
        sr: u32,
        /// The cutoff the current coefficients were computed for.
        prev_f: f32,
        b0: f32,
        b1: f32,
        b2: f32,
        a1: f32,
        a2: f32,
        x1: f32,
        x2: f32,
        y1: f32,
        y2: f32,
    },
    Delay {
        buf: Vec<f32>,
        w: usize,
        feedback: f32,
    },
    Reverb {
        combs: Vec<(Vec<f32>, usize, f32)>,
        allpasses: Vec<(Vec<f32>, usize)>,
        feedback: f32,
        damp: f32,
        g: f32,
        comb_norm: f32,
        mix: f32,
    },
    Modal {
        modes: Vec<ModalMode>,
        mix: f32,
    },
    Drive {
        amount: Val,
        shape: DriveShape,
        adaa: bool,
        x_prev: f32,
        f_prev: f32,
        dc_x: f32,
        dc_y: f32,
    },
    Tremolo {
        rate: f32,
        depth: f32,
        srf: f32,
    },
    Chorus {
        buf: Vec<f32>,
        w: usize,
        base: f32,
        swing: f32,
        max_delay: usize,
        mix: f32,
        rate: f32,
        srf: f32,
    },
    Compress {
        env: f32,
        at: f32,
        rt: f32,
        threshold: f32,
        ratio: f32,
        makeup: f32,
    },
}

impl Proc {
    /// Transform the samples at `t0..t0 + buf.len()` in place.
    pub(super) fn process(&mut self, buf: &mut [f32], t0: usize) {
        match self {
            Proc::Gain(amount) => {
                for (i, x) in buf.iter_mut().enumerate() {
                    *x *= amount.eval(t0 + i);
                }
            }
            Proc::Biquad {
                kind,
                cutoff,
                scale,
                q,
                sr,
                prev_f,
                b0,
                b1,
                b2,
                a1,
                a2,
                x1,
                x2,
                y1,
                y2,
            } => {
                for (i, x) in buf.iter_mut().enumerate() {
                    let f = cutoff.eval(t0 + i) * *scale;
                    if f != *prev_f {
                        (*b0, *b1, *b2, *a1, *a2) = biquad_coeffs(*kind, f, *q, *sr);
                        *prev_f = f;
                    }
                    let x0 = *x;
                    let y0 = *b0 * x0 + *b1 * *x1 + *b2 * *x2 - *a1 * *y1 - *a2 * *y2;
                    *x2 = *x1;
                    *x1 = x0;
                    *y2 = *y1;
                    *y1 = y0;
                    *x = y0;
                }
            }
            Proc::Delay {
                buf: line,
                w,
                feedback,
            } => {
                for x in buf.iter_mut() {
                    let delayed = line[*w];
                    let y = *x + *feedback * delayed;
                    line[*w] = y;
                    *w = (*w + 1) % line.len();
                    *x = y;
                }
            }
            Proc::Reverb {
                combs,
                allpasses,
                feedback,
                damp,
                g,
                comb_norm,
                mix,
            } => {
                for x in buf.iter_mut() {
                    let x0 = *x;
                    let mut wet = 0.0f32;
                    for (line, idx, fs) in combs.iter_mut() {
                        let y = line[*idx];
                        *fs = y * (1.0 - *damp) + *fs * *damp;
                        line[*idx] = x0 + *fs * *feedback;
                        *idx = (*idx + 1) % line.len();
                        wet += y;
                    }
                    for (line, idx) in allpasses.iter_mut() {
                        let buffered = line[*idx];
                        let y = -wet + buffered;
                        line[*idx] = wet + buffered * *g;
                        *idx = (*idx + 1) % line.len();
                        wet = y;
                    }
                    *x = x0 * (1.0 - *mix) + (wet * *comb_norm) * *mix;
                }
            }
            Proc::Modal { modes, mix } => {
                for x in buf.iter_mut() {
                    let x0 = *x;
                    let mut wet = 0.0f32;
                    for m in modes.iter_mut() {
                        let y = m.b0 * x0 + m.a1 * m.y1 + m.a2 * m.y2;
                        m.y2 = m.y1;
                        m.y1 = y;
                        wet += y;
                    }
                    *x = x0 * (1.0 - *mix) + wet * *mix;
                }
            }
            Proc::Drive {
                amount,
                shape,
                adaa,
                x_prev,
                f_prev,
                dc_x,
                dc_y,
            } => {
                // ADAA: the average of the curve over [x[n-1], x[n]] band-limits
                // the foldback; the midpoint fallback avoids the 0/0 when
                // consecutive inputs are nearly equal. A ~5 Hz one-pole DC
                // blocker follows, since the difference quotient leaves a
                // small DC term on asymmetric input.
                const EPS: f32 = 1e-5;
                const R: f32 = 0.9995;
                for (i, x) in buf.iter_mut().enumerate() {
                    let amt = amount.eval(t0 + i).max(0.0);
                    if !*adaa {
                        *x = drive_curve(amt * *x, *shape);
                        continue;
                    }
                    let xn = amt * *x;
                    let f = drive_antideriv(xn, *shape);
                    let d = xn - *x_prev;
                    let y = if d.abs() > EPS {
                        (f - *f_prev) / d
                    } else {
                        drive_curve(0.5 * (xn + *x_prev), *shape)
                    };
                    *x_prev = xn;
                    *f_prev = f;
                    let yb = y - *dc_x + R * *dc_y;
                    *dc_x = y;
                    *dc_y = yb;
                    *x = yb;
                }
            }
            Proc::Tremolo { rate, depth, srf } => {
                // Closed form of the absolute sample index: block-size
                // independent by construction.
                for (i, x) in buf.iter_mut().enumerate() {
                    let t = (t0 + i) as f32;
                    *x *= 1.0 - *depth * (0.5 + 0.5 * dsp::sin(TAU * *rate * t / *srf));
                }
            }
            Proc::Chorus {
                buf: line,
                w,
                base,
                swing,
                max_delay,
                mix,
                rate,
                srf,
            } => {
                for (i, x) in buf.iter_mut().enumerate() {
                    let x0 = *x;
                    line[*w] = x0;
                    let lfo = dsp::sin(TAU * *rate * (t0 + i) as f32 / *srf);
                    let delay = *base + *swing * lfo;
                    // Fractional read via linear interpolation.
                    let read = (*w as f32 - delay).rem_euclid(*max_delay as f32);
                    let i0 = read.floor() as usize % *max_delay;
                    let i1 = (i0 + 1) % *max_delay;
                    let frac = read - read.floor();
                    let wet = line[i0] * (1.0 - frac) + line[i1] * frac;
                    *x = x0 * (1.0 - *mix) + wet * *mix;
                    *w = (*w + 1) % *max_delay;
                }
            }
            Proc::Compress {
                env,
                at,
                rt,
                threshold,
                ratio,
                makeup,
            } => {
                for x in buf.iter_mut() {
                    let rect = x.abs();
                    // Attack when rising, release when falling.
                    let coeff = if rect > *env { *at } else { *rt };
                    *env = rect + coeff * (*env - rect);
                    let env_db = 20.0 * dsp::log10(env.max(1e-9));
                    let gain_db = if env_db > *threshold {
                        -(env_db - *threshold) * (1.0 - 1.0 / *ratio)
                    } else {
                        0.0
                    };
                    let g = dsp::powf(10.0, gain_db / 20.0);
                    *x = *x * g * *makeup;
                }
            }
        }
    }

    /// Apply a live cutoff `scale` (1.0 = as built) to a biquad, preserving
    /// its state so a sweep is click-free; the coefficients follow on the
    /// next sample. Every other processor ignores it.
    pub(super) fn set_cutoff(&mut self, scale: f32) {
        if let Proc::Biquad {
            scale: s, prev_f, ..
        } = self
        {
            *s = scale;
            *prev_f = f32::NAN;
        }
    }
}

/// A biquad over the authored (possibly modulated) cutoff; the coefficients
/// are computed on the first sample.
fn biquad(kind: FilterKind, cutoff: Val, q: f32, sr: u32) -> Proc {
    Proc::Biquad {
        kind,
        cutoff,
        scale: 1.0,
        q,
        sr,
        prev_f: f32::NAN,
        b0: 0.0,
        b1: 0.0,
        b2: 0.0,
        a1: 0.0,
        a2: 0.0,
        x1: 0.0,
        x2: 0.0,
        y1: 0.0,
        y2: 0.0,
    }
}

/// A Schroeder reverb at `sr` with `spread` extra samples per delay line —
/// the stereo decorrelation the mixer gives bus/master reverbs (left 0,
/// right 23).
pub(super) fn reverb_proc(room: f32, mix: f32, sr: u32, spread: usize) -> Proc {
    let (comb_lens, allpass_lens) = crate::dsp::freeverb_lengths(sr, spread);
    Proc::Reverb {
        combs: comb_lens
            .iter()
            .map(|&len| (vec![0.0f32; len], 0usize, 0.0f32))
            .collect(),
        allpasses: allpass_lens
            .iter()
            .map(|&len| (vec![0.0f32; len], 0usize))
            .collect(),
        feedback: crate::dsp::freeverb_feedback(room),
        damp: crate::dsp::FREEVERB_DAMP,
        g: 0.5,
        comb_norm: 1.0 / comb_lens.len() as f32,
        mix: mix.clamp(0.0, 1.0),
    }
}

/// Build the processor for `node`, or `None` for a node that is not a
/// processor.
pub(super) fn try_proc(node: &Node, ctx: &Ctx, _path: u64) -> Option<Proc> {
    let sr = ctx.sr;
    let srf = ctx.srf();
    let v = |val: &crate::dsl::Value| Val::build(val, sr, ctx.n);
    Some(match node {
        Node::Gain { amount } => Proc::Gain(v(amount)),
        Node::Lowpass { cutoff, q } => biquad(FilterKind::Low, v(cutoff), *q, sr),
        Node::Highpass { cutoff, q } => biquad(FilterKind::High, v(cutoff), *q, sr),
        Node::Bandpass { cutoff, q } => biquad(FilterKind::Band, v(cutoff), *q, sr),
        Node::Notch { cutoff, q } => biquad(FilterKind::Notch, v(cutoff), *q, sr),
        Node::Peak { cutoff, q, gain_db } => biquad(FilterKind::Peak(*gain_db), v(cutoff), *q, sr),
        Node::Lowshelf { cutoff, gain_db } => {
            biquad(FilterKind::LowShelf(*gain_db), v(cutoff), 0.707, sr)
        }
        Node::Highshelf { cutoff, gain_db } => {
            biquad(FilterKind::HighShelf(*gain_db), v(cutoff), 0.707, sr)
        }
        Node::Delay { secs, feedback } => {
            // validate() caps secs at 30 s; the shared clamp guards an
            // unvalidated doc from an unbounded allocation.
            let dn = crate::dsp::delay_line_len(*secs, sr);
            Proc::Delay {
                buf: vec![0.0; dn],
                w: 0,
                feedback: *feedback,
            }
        }
        Node::Reverb { room, mix } => reverb_proc(*room, *mix, sr, 0),
        Node::Modal { modes, mix } => {
            let modes = modes
                .iter()
                .map(|m| {
                    let (a1, a2, b0) = crate::dsp::modal_coeffs(m.freq, m.decay, m.gain, sr);
                    ModalMode {
                        a1,
                        a2,
                        b0,
                        y1: 0.0,
                        y2: 0.0,
                    }
                })
                .collect();
            Proc::Modal {
                modes,
                mix: mix.clamp(0.0, 1.0),
            }
        }
        Node::Drive { amount, shape, aa } => Proc::Drive {
            amount: v(amount),
            shape: *shape,
            // ADAA is on unless the author selects the pointwise curve.
            adaa: aa.unwrap_or(true),
            x_prev: 0.0,
            f_prev: drive_antideriv(0.0, *shape),
            dc_x: 0.0,
            dc_y: 0.0,
        },
        Node::Tremolo { rate, depth } => Proc::Tremolo {
            rate: *rate,
            depth: *depth,
            srf,
        },
        Node::Chorus { rate, depth, mix } => {
            let base = crate::dsp::CHORUS_BASE_SECS * srf;
            let swing = depth.clamp(0.0, 1.0) * crate::dsp::CHORUS_SWING_SECS * srf;
            let max_delay = (base + swing) as usize + 2;
            Proc::Chorus {
                buf: vec![0.0; max_delay],
                w: 0,
                base,
                swing,
                max_delay,
                mix: mix.clamp(0.0, 1.0),
                rate: *rate,
                srf,
            }
        }
        Node::Compress {
            threshold,
            ratio,
            attack,
            release,
            makeup,
        } => {
            let (at, rt) = dsp::follower_coeffs(*attack, *release, srf);
            Proc::Compress {
                env: 0.0,
                at,
                rt,
                threshold: *threshold,
                ratio: ratio.max(1.0),
                makeup: dsp::powf(10.0, makeup / 20.0),
            }
        }
        _ => return None,
    })
}
