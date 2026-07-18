//! Streaming processors: the stateful per-sample evaluators of every
//! transforming node (filters, delays, dynamics, waveshaping), and the graph
//! walk that builds them.

use std::f32::consts::TAU;

use super::source::{Src, try_src};
use super::value::Val;
use crate::dsl::{DriveShape, Node, Value, note_to_hz};
use crate::dsp::node_path;
use crate::render::{FilterKind, biquad_coeffs, drive_antideriv, drive_curve};

/// One resonator of a [`Proc::Modal`] bank: constant LTI coeffs + 2-pole state.
pub(super) struct ModalMode {
    a1: f32,
    a2: f32,
    b0: f32,
    y1: f32,
    y2: f32,
}

/// A streamable processor, holding its per-sample state.
pub(super) enum Proc {
    Gain(f32),
    Biquad {
        // The filter spec, kept so a live cutoff sweep can recompute the
        // coefficients without a rebuild (see `Proc::set_cutoff`).
        kind: FilterKind,
        fc: f32,
        q: f32,
        sr: u32,
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
    Bitcrush {
        half: f32,
    },
    Downsample {
        f: usize,
        held: f32,
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
    RingMod {
        phase: f32,
        freq: Val,
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
    Flanger {
        buf: Vec<f32>,
        w: usize,
        base: f32,
        swing: f32,
        max_delay: usize,
        fb: f32,
        mix: f32,
        rate: f32,
        srf: f32,
    },
    Phaser {
        x1: [f32; 4],
        y1: [f32; 4],
        last_wet: f32,
        rate: f32,
        depth: f32,
        fb: f32,
        mix: f32,
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
    Duck {
        trigger: Box<Src>,
        env: f32,
        at: f32,
        rt: f32,
        amount: f32,
    },
}

impl Proc {
    /// Process one sample. `pitch` is forwarded so pitch-tracking processors (the
    /// ring-mod carrier, a `duck` sidechain source) bend with the note; every
    /// other processor ignores it. Bit-identical to offline at `pitch == 1.0`.
    pub(super) fn step(&mut self, x0: f32, t: usize, pitch: f32) -> f32 {
        match self {
            Proc::Gain(a) => x0 * *a,
            Proc::Biquad {
                b0,
                b1,
                b2,
                a1,
                a2,
                x1,
                x2,
                y1,
                y2,
                ..
            } => {
                let y0 = *b0 * x0 + *b1 * *x1 + *b2 * *x2 - *a1 * *y1 - *a2 * *y2;
                *x2 = *x1;
                *x1 = x0;
                *y2 = *y1;
                *y1 = y0;
                y0
            }
            Proc::Bitcrush { half } => (x0.clamp(-1.0, 1.0) * *half).round() / *half,
            Proc::Downsample { f, held } => {
                if t.is_multiple_of(*f) {
                    *held = x0;
                }
                *held
            }
            Proc::Delay { buf, w, feedback } => {
                let delayed = buf[*w];
                let y = x0 + *feedback * delayed;
                buf[*w] = y;
                *w = (*w + 1) % buf.len();
                y
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
                let mut wet = 0.0f32;
                for (buf, idx, fs) in combs.iter_mut() {
                    let y = buf[*idx];
                    *fs = y * (1.0 - *damp) + *fs * *damp;
                    buf[*idx] = x0 + *fs * *feedback;
                    *idx = (*idx + 1) % buf.len();
                    wet += y;
                }
                for (buf, idx) in allpasses.iter_mut() {
                    let buffered = buf[*idx];
                    let y = -wet + buffered;
                    buf[*idx] = wet + buffered * *g;
                    *idx = (*idx + 1) % buf.len();
                    wet = y;
                }
                x0 * (1.0 - *mix) + (wet * *comb_norm) * *mix
            }
            Proc::Modal { modes, mix } => {
                let mut wet = 0.0f32;
                for m in modes.iter_mut() {
                    let y = m.b0 * x0 + m.a1 * m.y1 + m.a2 * m.y2;
                    m.y2 = m.y1;
                    m.y1 = y;
                    wet += y;
                }
                x0 * (1.0 - *mix) + wet * *mix
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
                let amt = amount.eval(t).max(0.0);
                if !*adaa {
                    drive_curve(amt * x0, *shape)
                } else {
                    const EPS: f32 = 1e-5;
                    const R: f32 = 0.9995;
                    let xn = amt * x0;
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
                    yb
                }
            }
            Proc::RingMod { phase, freq, srf } => {
                let out = x0 * (TAU * *phase).sin();
                *phase += freq.eval(t).max(0.0) * pitch / *srf;
                *phase -= phase.floor();
                out
            }
            Proc::Chorus {
                buf,
                w,
                base,
                swing,
                max_delay,
                mix,
                rate,
                srf,
            } => {
                buf[*w] = x0;
                let lfo = (TAU * *rate * t as f32 / *srf).sin();
                let delay = *base + *swing * lfo;
                let read = (*w as f32 - delay).rem_euclid(*max_delay as f32);
                let i0 = read.floor() as usize % *max_delay;
                let i1 = (i0 + 1) % *max_delay;
                let frac = read - read.floor();
                let wet = buf[i0] * (1.0 - frac) + buf[i1] * frac;
                let out = x0 * (1.0 - *mix) + wet * *mix;
                *w = (*w + 1) % *max_delay;
                out
            }
            Proc::Flanger {
                buf,
                w,
                base,
                swing,
                max_delay,
                fb,
                mix,
                rate,
                srf,
            } => {
                let lfo = (TAU * *rate * t as f32 / *srf).sin();
                let delay = *base + *swing * lfo;
                let read = (*w as f32 - delay).rem_euclid(*max_delay as f32);
                let i0 = read.floor() as usize % *max_delay;
                let i1 = (i0 + 1) % *max_delay;
                let frac = read - read.floor();
                let wet = buf[i0] * (1.0 - frac) + buf[i1] * frac;
                buf[*w] = x0 + wet * *fb;
                *w = (*w + 1) % *max_delay;
                x0 * (1.0 - *mix) + wet * *mix
            }
            Proc::Phaser {
                x1,
                y1,
                last_wet,
                rate,
                depth,
                fb,
                mix,
                srf,
            } => {
                let lfo = 0.5 + 0.5 * (TAU * *rate * t as f32 / *srf).sin();
                let g = 0.15 + 0.7 * *depth * lfo;
                let mut s = x0 + *last_wet * *fb;
                for k in 0..4 {
                    let y = -g * s + x1[k] + g * y1[k];
                    x1[k] = s;
                    y1[k] = y;
                    s = y;
                }
                *last_wet = s;
                x0 * (1.0 - *mix) + s * *mix
            }
            Proc::Compress {
                env,
                at,
                rt,
                threshold,
                ratio,
                makeup,
            } => {
                let rect = x0.abs();
                let coeff = if rect > *env { *at } else { *rt };
                *env = rect + coeff * (*env - rect);
                let env_db = 20.0 * env.max(1e-9).log10();
                let gain_db = if env_db > *threshold {
                    -(env_db - *threshold) * (1.0 - 1.0 / *ratio)
                } else {
                    0.0
                };
                let g = 10f32.powf(gain_db / 20.0);
                x0 * g * *makeup
            }
            Proc::Duck {
                trigger,
                env,
                at,
                rt,
                amount,
            } => {
                let trig = trigger.step(t, pitch);
                let rect = trig.abs().min(1.0);
                let coeff = if rect > *env { *at } else { *rt };
                *env = rect + coeff * (*env - rect);
                x0 * (1.0 - *amount * *env)
            }
        }
    }

    /// Recompute a biquad's coefficients for a live cutoff `scale` (1.0 = as
    /// built), preserving filter state so a sweep is click-free. Only biquad
    /// filters respond; every other processor ignores it. At `scale == 1.0` the
    /// coefficients are bit-identical to the baked ones.
    pub(super) fn set_cutoff(&mut self, scale: f32) {
        if let Proc::Biquad {
            kind,
            fc,
            q,
            sr,
            b0,
            b1,
            b2,
            a1,
            a2,
            ..
        } = self
        {
            let (nb0, nb1, nb2, na1, na2) = biquad_coeffs(*kind, *fc * scale, *q, *sr);
            *b0 = nb0;
            *b1 = nb1;
            *b2 = nb2;
            *a1 = na1;
            *a2 = na2;
        }
    }
}

/// Build a streaming biquad, keeping its spec so [`Proc::set_cutoff`] can
/// recompute the coefficients for a live cutoff sweep. The coefficients come
/// from the offline renderer's own [`biquad_coeffs`] table — one table, both
/// paths, byte-identical by construction.
fn biquad(kind: FilterKind, fc: f32, q: f32, sr: u32) -> Proc {
    let (b0, b1, b2, a1, a2) = biquad_coeffs(kind, fc, q, sr);
    Proc::Biquad {
        kind,
        fc,
        q,
        sr,
        b0,
        b1,
        b2,
        a1,
        a2,
        x1: 0.0,
        x2: 0.0,
        y1: 0.0,
        y2: 0.0,
    }
}

pub(super) fn try_proc(node: &Node, sr: u32, n: usize, engine: u32, path: u64) -> Option<Proc> {
    let srf = sr as f32;
    // Filters/EQ only stream with a constant cutoff.
    let cst = |val: &Value| match val {
        Value::Const(c) => Some(*c),
        Value::Note(s) => Some(note_to_hz(s).unwrap_or(440.0)),
        Value::Modulated(_) => None,
    };
    let v = |val: &Value| Val::build(val, sr, n);
    Some(match node {
        Node::Gain { amount } => Proc::Gain(cst(amount)?),
        Node::Lowpass { cutoff, q } => biquad(FilterKind::Low, cst(cutoff)?, *q, sr),
        Node::Highpass { cutoff, q } => biquad(FilterKind::High, cst(cutoff)?, *q, sr),
        Node::Bandpass { cutoff, q } => biquad(FilterKind::Band, cst(cutoff)?, *q, sr),
        Node::Notch { cutoff, q } => biquad(FilterKind::Notch, cst(cutoff)?, *q, sr),
        Node::Peak { cutoff, q, gain_db } => {
            biquad(FilterKind::Peak(*gain_db), cst(cutoff)?, *q, sr)
        }
        Node::Lowshelf { cutoff, gain_db } => {
            biquad(FilterKind::LowShelf(*gain_db), cst(cutoff)?, 0.707, sr)
        }
        Node::Highshelf { cutoff, gain_db } => {
            biquad(FilterKind::HighShelf(*gain_db), cst(cutoff)?, 0.707, sr)
        }
        Node::Bitcrush { bits } => {
            // Mirrors the offline clamp: validate() bounds bits to 1..=16;
            // .min(31) keeps an unvalidated doc from overflowing the shift.
            let levels = (1u32 << (*bits as u32).min(31)) as f32;
            Proc::Bitcrush { half: levels / 2.0 }
        }
        Node::Downsample { factor } => Proc::Downsample {
            f: (*factor).max(1) as usize,
            held: 0.0,
        },
        Node::Delay { secs, feedback } => {
            // Mirrors the offline clamp: validate() caps secs at 30 s; this
            // guards unvalidated docs from an unbounded allocation.
            let dn = ((secs.min(30.0) * srf) as usize).max(1);
            Proc::Delay {
                buf: vec![0.0; dn],
                w: 0,
                feedback: *feedback,
            }
        }
        Node::Reverb { room, mix } => {
            let scale = srf / 44_100.0;
            let comb_tunings = crate::dsp::FREEVERB_COMB_TUNINGS;
            let allpass_tunings = crate::dsp::FREEVERB_ALLPASS_TUNINGS;
            let combs = comb_tunings
                .iter()
                .map(|&tn| {
                    (
                        vec![0.0f32; ((tn as f32 * scale) as usize).max(1)],
                        0usize,
                        0.0f32,
                    )
                })
                .collect();
            let allpasses = allpass_tunings
                .iter()
                .map(|&tn| (vec![0.0f32; ((tn as f32 * scale) as usize).max(1)], 0usize))
                .collect();
            Proc::Reverb {
                combs,
                allpasses,
                feedback: 0.7 + 0.28 * room.clamp(0.0, 1.0),
                damp: crate::dsp::FREEVERB_DAMP,
                g: 0.5,
                comb_norm: 1.0 / 6.0,
                mix: mix.clamp(0.0, 1.0),
            }
        }
        Node::Modal { modes, mix } => {
            let nyq = srf * 0.5;
            let modes = modes
                .iter()
                .map(|m| {
                    // The .max(1.0) guard keeps the clamp ordered at absurd
                    // sample rates (sr < 4), matching the offline path.
                    let f0 = m.freq.clamp(1.0, (nyq - 1.0).max(1.0));
                    let decay = m.decay.max(1e-3);
                    let w0 = TAU * f0 / srf;
                    let (sin0, cos0) = (w0.sin(), w0.cos());
                    // r so the ring reaches −60 dB (×0.001) after `decay` seconds.
                    let r = (crate::dsp::NEG_LN_1000 / (decay * srf)).exp();
                    ModalMode {
                        a1: 2.0 * r * cos0,
                        a2: -r * r,
                        b0: m.gain * sin0,
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
            adaa: engine >= 1 && aa.unwrap_or(true),
            x_prev: 0.0,
            f_prev: drive_antideriv(0.0, *shape),
            dc_x: 0.0,
            dc_y: 0.0,
        },
        Node::RingMod { freq } => Proc::RingMod {
            phase: 0.0,
            freq: v(freq),
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
        Node::Flanger {
            rate,
            depth,
            feedback,
            mix,
        } => {
            let base = crate::dsp::FLANGER_BASE_SECS * srf;
            let swing = depth.clamp(0.0, 1.0) * crate::dsp::FLANGER_SWING_SECS * srf;
            let max_delay = (base + swing) as usize + 2;
            Proc::Flanger {
                buf: vec![0.0; max_delay],
                w: 0,
                base,
                swing,
                max_delay,
                fb: feedback.clamp(0.0, 0.95),
                mix: mix.clamp(0.0, 1.0),
                rate: *rate,
                srf,
            }
        }
        Node::Phaser {
            rate,
            depth,
            feedback,
            mix,
        } => Proc::Phaser {
            x1: [0.0; 4],
            y1: [0.0; 4],
            last_wet: 0.0,
            rate: *rate,
            depth: depth.clamp(0.0, 1.0),
            fb: feedback.clamp(0.0, 0.95),
            mix: mix.clamp(0.0, 1.0),
            srf,
        },
        Node::Compress {
            threshold,
            ratio,
            attack,
            release,
            makeup,
        } => Proc::Compress {
            env: 0.0,
            at: (-1.0 / (attack.max(1e-4) * srf)).exp(),
            rt: (-1.0 / (release.max(1e-4) * srf)).exp(),
            threshold: *threshold,
            ratio: ratio.max(1.0),
            makeup: 10f32.powf(makeup / 20.0),
        },
        Node::Duck {
            trigger,
            amount,
            attack,
            release,
        } => Proc::Duck {
            trigger: Box::new(try_src(trigger, sr, n, engine, node_path(path, 0))?),
            env: 0.0,
            at: (-1.0 / (attack.max(1e-4) * srf)).exp(),
            rt: (-1.0 / (release.max(1e-4) * srf)).exp(),
            amount: *amount,
        },
        _ => return None,
    })
}
