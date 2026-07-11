//! Streaming sources: the stateful per-sample evaluators of every signal-
//! producing node, and the graph walk that builds them.

use std::f32::consts::TAU;

use super::proc::{Proc, try_proc};
use super::value::Val;
use crate::dsl::{Node, NoiseColor, SeqWave, Shape, SuperWave, Value};
use crate::dsp::{Rng, adsr_env, node_path, node_seed};
use crate::render::{osc, poly_blep, seq_to_signal};

/// Per-color filter state for a streaming noise node.
pub(super) enum NoiseKind {
    White,
    Pink { b: [f32; 7] },
    Brown { last: f32 },
}

/// A streamable source / combinator node, holding its per-sample state.
pub(super) enum Src {
    Sine {
        phase: f32,
        freq: Val,
        srf: f32,
    },
    Square {
        phase: f32,
        freq: Val,
        duty: Val,
        srf: f32,
    },
    Saw {
        phase: f32,
        freq: Val,
        srf: f32,
    },
    Tri {
        phase: f32,
        tri: f32,
        freq: Val,
        srf: f32,
    },
    Fm {
        cph: f32,
        mph: f32,
        freq: Val,
        index: Val,
        ratio: f32,
        srf: f32,
    },
    Super {
        wave: SuperWave,
        freq: Val,
        phases: Vec<f32>,
        ratios: Vec<f32>,
        scale: f32,
        srf: f32,
    },
    Impact {
        w: usize,
        norm: f32,
    },
    Env {
        a: f32,
        d: f32,
        s: f32,
        r: f32,
        punch: f32,
        srf: f32,
        rel_start: f32,
    },
    /// `engine >= 2` only: a structurally-seeded noise leaf (own RNG).
    Noise {
        rng: Rng,
        kind: NoiseKind,
    },
    /// `engine >= 2` only: a structurally-seeded dust leaf.
    Dust {
        rng: Rng,
        p: f32,
        g: f32,
        y: f32,
    },
    /// `engine >= 2`, non-sampler only: a seq pre-rendered (via the exact offline
    /// synthesis, structurally seeded) and read back block-by-block.
    Seq {
        buf: Vec<f32>,
    },
    Mix(Vec<Src>),
    Mul(Vec<Src>),
    Chain {
        src: Box<Src>,
        procs: Vec<Proc>,
    },
}

impl Src {
    /// The next sample at absolute index `t`. Mirrors the offline per-sample math.
    /// `pitch` is a live multiplier applied to every oscillator frequency (1.0 =
    /// as authored) — it drives pitch bend and glide without rebuilding the graph.
    /// At `pitch == 1.0` the arithmetic is bit-identical to the offline render.
    pub(super) fn step(&mut self, t: usize, pitch: f32) -> f32 {
        match self {
            Src::Sine { phase, freq, srf } => {
                let v = osc(Shape::Sine, *phase);
                *phase += freq.eval(t).max(0.0) * pitch / *srf;
                *phase -= phase.floor();
                v
            }
            Src::Square {
                phase,
                freq,
                duty,
                srf,
            } => {
                let dt = freq.eval(t).max(0.0) * pitch / *srf;
                let du = duty.eval(t).clamp(0.01, 0.99);
                let mut v = if *phase < du { 1.0 } else { -1.0 };
                v += poly_blep(*phase, dt);
                v -= poly_blep((*phase - du + 1.0).fract(), dt);
                *phase += dt;
                *phase -= phase.floor();
                v
            }
            Src::Saw { phase, freq, srf } => {
                let dt = freq.eval(t).max(0.0) * pitch / *srf;
                let v = (2.0 * *phase - 1.0) - poly_blep(*phase, dt);
                *phase += dt;
                *phase -= phase.floor();
                v
            }
            Src::Tri {
                phase,
                tri,
                freq,
                srf,
            } => {
                let dt = freq.eval(t).max(0.0) * pitch / *srf;
                let mut sq = if *phase < 0.5 { 1.0 } else { -1.0 };
                sq += poly_blep(*phase, dt);
                sq -= poly_blep((*phase + 0.5).fract(), dt);
                *tri = *tri * 0.9995 + 4.0 * dt * sq;
                let v = *tri;
                *phase += dt;
                *phase -= phase.floor();
                v
            }
            Src::Fm {
                cph,
                mph,
                freq,
                index,
                ratio,
                srf,
            } => {
                let m = index.eval(t) * (TAU * *mph).sin();
                let y = (TAU * *cph + m).sin();
                let fi = freq.eval(t).max(0.0) * pitch;
                *cph += fi / *srf;
                *cph -= cph.floor();
                *mph += (fi * *ratio) / *srf;
                *mph -= mph.floor();
                y
            }
            Src::Super {
                wave,
                freq,
                phases,
                ratios,
                scale,
                srf,
            } => {
                let f = freq.eval(t).max(0.0) * pitch;
                let mut acc = 0.0f32;
                for k in 0..phases.len() {
                    let dt = f * ratios[k] / *srf;
                    let s = match wave {
                        SuperWave::Sawtooth => (2.0 * phases[k] - 1.0) - poly_blep(phases[k], dt),
                        SuperWave::Square => {
                            let mut sq = if phases[k] < 0.5 { 1.0 } else { -1.0 };
                            sq += poly_blep(phases[k], dt);
                            sq -= poly_blep((phases[k] + 0.5).fract(), dt);
                            sq
                        }
                    };
                    acc += s;
                    phases[k] += dt;
                    phases[k] -= phases[k].floor();
                }
                acc * *scale
            }
            Src::Impact { w, norm } => {
                if t < *w {
                    let phase = (t as f32 + 0.5) / *w as f32;
                    *norm * 0.5 * (1.0 - (TAU * phase).cos())
                } else {
                    0.0
                }
            }
            Src::Env {
                a,
                d,
                s,
                r,
                punch,
                srf,
                rel_start,
            } => adsr_env(t as f32 / *srf, *a, *d, *s, *r, *punch, *rel_start),
            Src::Noise { rng, kind } => match kind {
                NoiseKind::White => rng.bi(),
                NoiseKind::Pink { b } => {
                    let w = rng.bi();
                    b[0] = 0.99886 * b[0] + w * 0.0555179;
                    b[1] = 0.99332 * b[1] + w * 0.0750759;
                    b[2] = 0.96900 * b[2] + w * 0.153_852;
                    b[3] = 0.86650 * b[3] + w * 0.3104856;
                    b[4] = 0.55000 * b[4] + w * 0.5329522;
                    b[5] = -0.7616 * b[5] - w * 0.0168980;
                    let out = b[0] + b[1] + b[2] + b[3] + b[4] + b[5] + b[6] + w * 0.5362;
                    b[6] = w * 0.115926;
                    out * 0.11
                }
                NoiseKind::Brown { last } => {
                    *last = (*last + 0.02 * rng.bi()) * 0.998;
                    (*last * 8.0).clamp(-1.0, 1.0)
                }
            },
            Src::Dust { rng, p, g, y } => {
                let imp = if rng.unit() < *p { rng.bi() } else { 0.0 };
                *y = imp + *g * *y;
                *y
            }
            Src::Seq { buf } => buf.get(t).copied().unwrap_or(0.0),
            Src::Mix(cs) => {
                let mut acc = 0.0f32;
                for c in cs.iter_mut() {
                    acc += c.step(t, pitch);
                }
                acc
            }
            Src::Mul(cs) => {
                let mut acc = 1.0f32;
                for c in cs.iter_mut() {
                    acc *= c.step(t, pitch);
                }
                acc
            }
            Src::Chain { src, procs } => {
                let mut x = src.step(t, pitch);
                for p in procs.iter_mut() {
                    x = p.step(x, t, pitch);
                }
                x
            }
        }
    }

    /// Apply a live cutoff `scale` to every biquad filter in the signal tree —
    /// the instrument brightness control. Recurses through mix/mul/chain.
    pub(super) fn set_cutoff(&mut self, scale: f32) {
        match self {
            Src::Mix(cs) | Src::Mul(cs) => cs.iter_mut().for_each(|c| c.set_cutoff(scale)),
            Src::Chain { src, procs } => {
                src.set_cutoff(scale);
                procs.iter_mut().for_each(|p| p.set_cutoff(scale));
            }
            _ => {}
        }
    }
}

pub(super) fn try_src(node: &Node, sr: u32, n: usize, engine: u32, path: u64) -> Option<Src> {
    let srf = sr as f32;
    let v = |val: &Value| Val::build(val, sr, n);
    Some(match node {
        // Engine >= 2: noise/dust own a structurally-seeded RNG (from `path`),
        // exactly as the offline render_node does under engine >= 2, so the
        // streamed randomness is byte-identical.
        Node::Noise { color } if engine >= 2 => Src::Noise {
            rng: Rng::new(node_seed(path)),
            kind: match color {
                NoiseColor::White => NoiseKind::White,
                NoiseColor::Pink => NoiseKind::Pink { b: [0.0; 7] },
                NoiseColor::Brown => NoiseKind::Brown { last: 0.0 },
            },
        },
        Node::Dust { density, decay } if engine >= 2 => {
            let p = (density / srf).clamp(0.0, 1.0);
            let g = if *decay > 0.0 {
                (-1.0 / (decay * srf)).exp()
            } else {
                0.0
            };
            Src::Dust {
                rng: Rng::new(node_seed(path)),
                p,
                g,
                y: 0.0,
            }
        }
        // Non-sampler seq (engine >= 2): pre-render with a structurally-seeded RNG
        // (the exact offline synthesis) and read it back block-by-block. Sampler
        // seq is external-synth-coupled and stays on the buffered fallback.
        Node::Seq { wave, .. } if engine >= 2 && *wave != SeqWave::Sampler => Src::Seq {
            buf: seq_to_signal(node, n, sr, &mut Rng::new(node_seed(path)), engine),
        },
        Node::Sine { freq } => Src::Sine {
            phase: 0.0,
            freq: v(freq),
            srf,
        },
        Node::Square { freq, duty } => Src::Square {
            phase: 0.0,
            freq: v(freq),
            duty: v(duty),
            srf,
        },
        Node::Sawtooth { freq } => Src::Saw {
            phase: 0.0,
            freq: v(freq),
            srf,
        },
        Node::Triangle { freq } => Src::Tri {
            phase: 0.0,
            tri: 0.0,
            freq: v(freq),
            srf,
        },
        Node::Fm { freq, ratio, index } => Src::Fm {
            cph: 0.0,
            mph: 0.0,
            freq: v(freq),
            index: v(index),
            ratio: *ratio,
            srf,
        },
        Node::Super {
            wave,
            freq,
            voices,
            detune_cents,
        } => {
            let count = (*voices).clamp(1, 16) as usize;
            let mut phases = Vec::with_capacity(count);
            let mut ratios = Vec::with_capacity(count);
            for k in 0..count {
                phases.push(k as f32 / count as f32);
                let cents = if count == 1 {
                    0.0
                } else {
                    -detune_cents + 2.0 * detune_cents * (k as f32 / (count as f32 - 1.0))
                };
                ratios.push(2f32.powf(cents / 1200.0));
            }
            Src::Super {
                wave: *wave,
                freq: v(freq),
                phases,
                ratios,
                scale: 1.0 / count as f32,
                srf,
            }
        }
        Node::Impact { hardness, velocity } => {
            let h = hardness.clamp(0.0, 1.0);
            let vel = velocity.clamp(0.0, 1.0);
            let width_s = 0.008 * (1.0 - h) + 0.0003 * h;
            let w = ((width_s * srf).round() as usize).max(1);
            Src::Impact {
                w,
                norm: vel / (0.5 * w as f32),
            }
        }
        Node::Env { adsr } => Src::Env {
            a: adsr.a,
            d: adsr.d,
            s: adsr.s,
            r: adsr.r,
            punch: adsr.punch,
            srf,
            rel_start: (n as f32 / srf - adsr.r).max(0.0),
        },
        Node::Mix { inputs } => Src::Mix(
            inputs
                .iter()
                .enumerate()
                .map(|(i, c)| try_src(c, sr, n, engine, node_path(path, i)))
                .collect::<Option<_>>()?,
        ),
        Node::Mul { inputs } => Src::Mul(
            inputs
                .iter()
                .enumerate()
                .map(|(i, c)| try_src(c, sr, n, engine, node_path(path, i)))
                .collect::<Option<_>>()?,
        ),
        Node::Chain { stages } => {
            let (first, rest) = stages.split_first()?;
            let src = Box::new(try_src(first, sr, n, engine, node_path(path, 0))?);
            let procs = rest
                .iter()
                .enumerate()
                .map(|(i, p)| try_proc(p, sr, n, engine, node_path(path, i + 1)))
                .collect::<Option<_>>()?;
            Src::Chain { src, procs }
        }
        _ => return None,
    })
}
