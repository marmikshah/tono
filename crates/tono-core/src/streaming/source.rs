//! Sources: the stateful block evaluators of every signal-producing node and
//! the combinators, plus the graph walk that builds them.

use std::f32::consts::TAU;

use super::Ctx;
use super::proc::{Proc, try_proc};
use super::value::Val;
use crate::dsl::{Node, NoiseColor, SeqWave, Shape, SuperWave, Value};
use crate::dsp::{Rng, adsr_env, node_path, node_seed};
use crate::render::{osc, poly_blep, seq_to_signal, wavetable_frames, wavetable_lookup};

/// Per-color filter state for a noise node.
pub(super) enum NoiseKind {
    White,
    Pink { b: [f32; 7] },
    Brown { last: f32 },
}

/// A source or combinator node holding its state between blocks. `process`
/// evaluates the samples `t0..t0 + out.len()` in order; the arithmetic per
/// sample never depends on how the timeline was cut into blocks.
pub(super) enum Src {
    /// Exact zeros: a chain with no source stage, or a processor used where a
    /// source belongs.
    Silence,
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
    Wavetable {
        frames: Vec<Vec<f32>>,
        phase: f32,
        freq: Val,
        position: Val,
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
    /// A structurally-seeded noise leaf (own RNG).
    Noise {
        rng: Rng,
        kind: NoiseKind,
    },
    /// A structurally-seeded dust leaf.
    Dust {
        rng: Rng,
        p: f32,
        g: f32,
        y: f32,
    },
    /// A seq pre-rendered at build (the exact seq synthesis, structurally
    /// seeded) and read back block by block.
    Seq {
        buf: Vec<f32>,
    },
    Mix {
        inputs: Vec<Src>,
        /// One child's block, reused across the children.
        scratch: Vec<f32>,
    },
    Mul {
        inputs: Vec<Src>,
        scratch: Vec<f32>,
    },
    Chain {
        src: Box<Src>,
        procs: Vec<Proc>,
    },
}

impl Src {
    /// Write the samples at absolute indices `t0..t0 + out.len()`. `pitch` is
    /// the per-sample live multiplier on every oscillator frequency (1.0 = as
    /// authored), driving bend and glide without a rebuild; at 1.0 the
    /// arithmetic is the authored graph's.
    pub(super) fn process(&mut self, out: &mut [f32], t0: usize, pitch: &[f32]) {
        debug_assert_eq!(out.len(), pitch.len());
        match self {
            Src::Silence => out.fill(0.0),
            Src::Sine { phase, freq, srf } => {
                for (i, o) in out.iter_mut().enumerate() {
                    *o = osc(Shape::Sine, *phase);
                    *phase += freq.eval(t0 + i).max(0.0) * pitch[i] / *srf;
                    *phase -= phase.floor();
                }
            }
            Src::Square {
                phase,
                freq,
                duty,
                srf,
            } => {
                for (i, o) in out.iter_mut().enumerate() {
                    let t = t0 + i;
                    let dt = freq.eval(t).max(0.0) * pitch[i] / *srf;
                    let du = duty.eval(t).clamp(0.01, 0.99);
                    let mut v = if *phase < du { 1.0 } else { -1.0 };
                    v += poly_blep(*phase, dt);
                    v -= poly_blep((*phase - du + 1.0).fract(), dt);
                    *o = v;
                    *phase += dt;
                    *phase -= phase.floor();
                }
            }
            Src::Saw { phase, freq, srf } => {
                for (i, o) in out.iter_mut().enumerate() {
                    let dt = freq.eval(t0 + i).max(0.0) * pitch[i] / *srf;
                    *o = (2.0 * *phase - 1.0) - poly_blep(*phase, dt);
                    *phase += dt;
                    *phase -= phase.floor();
                }
            }
            Src::Tri {
                phase,
                tri,
                freq,
                srf,
            } => {
                for (i, o) in out.iter_mut().enumerate() {
                    let dt = freq.eval(t0 + i).max(0.0) * pitch[i] / *srf;
                    // Band-limited square (duty 0.5), integrated with a leak.
                    let mut sq = if *phase < 0.5 { 1.0 } else { -1.0 };
                    sq += poly_blep(*phase, dt);
                    sq -= poly_blep((*phase + 0.5).fract(), dt);
                    *tri = *tri * 0.9995 + 4.0 * dt * sq;
                    *o = *tri;
                    *phase += dt;
                    *phase -= phase.floor();
                }
            }
            Src::Fm {
                cph,
                mph,
                freq,
                index,
                ratio,
                srf,
            } => {
                for (i, o) in out.iter_mut().enumerate() {
                    let t = t0 + i;
                    let m = index.eval(t) * crate::dsp::sin(TAU * *mph);
                    *o = crate::dsp::sin(TAU * *cph + m);
                    let fi = freq.eval(t).max(0.0) * pitch[i];
                    *cph += fi / *srf;
                    *cph -= cph.floor();
                    *mph += (fi * *ratio) / *srf;
                    *mph -= mph.floor();
                }
            }
            Src::Super {
                wave,
                freq,
                phases,
                ratios,
                scale,
                srf,
            } => {
                for (i, o) in out.iter_mut().enumerate() {
                    let f = freq.eval(t0 + i).max(0.0) * pitch[i];
                    let mut acc = 0.0f32;
                    for k in 0..phases.len() {
                        let dt = f * ratios[k] / *srf;
                        let s = match wave {
                            SuperWave::Sawtooth => {
                                (2.0 * phases[k] - 1.0) - poly_blep(phases[k], dt)
                            }
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
                    *o = acc * *scale;
                }
            }
            Src::Wavetable {
                frames,
                phase,
                freq,
                position,
                srf,
            } => {
                for (i, o) in out.iter_mut().enumerate() {
                    let t = t0 + i;
                    *o = wavetable_lookup(frames, position.eval(t), *phase);
                    *phase += freq.eval(t).max(0.0) * pitch[i] / *srf;
                    *phase -= phase.floor();
                }
            }
            Src::Impact { w, norm } => {
                for (i, o) in out.iter_mut().enumerate() {
                    let t = t0 + i;
                    *o = if t < *w {
                        let phase = (t as f32 + 0.5) / *w as f32;
                        *norm * 0.5 * (1.0 - crate::dsp::cos(TAU * phase))
                    } else {
                        0.0
                    };
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
            } => {
                for (i, o) in out.iter_mut().enumerate() {
                    *o = adsr_env((t0 + i) as f32 / *srf, *a, *d, *s, *r, *punch, *rel_start);
                }
            }
            Src::Noise { rng, kind } => match kind {
                NoiseKind::White => {
                    for o in out.iter_mut() {
                        *o = rng.bi();
                    }
                }
                NoiseKind::Pink { b } => {
                    // Paul Kellet's economical pink-noise filter.
                    for o in out.iter_mut() {
                        let w = rng.bi();
                        b[0] = 0.99886 * b[0] + w * 0.0555179;
                        b[1] = 0.99332 * b[1] + w * 0.0750759;
                        b[2] = 0.96900 * b[2] + w * 0.153_852;
                        b[3] = 0.86650 * b[3] + w * 0.3104856;
                        b[4] = 0.55000 * b[4] + w * 0.5329522;
                        b[5] = -0.7616 * b[5] - w * 0.0168980;
                        let v = b[0] + b[1] + b[2] + b[3] + b[4] + b[5] + b[6] + w * 0.5362;
                        b[6] = w * 0.115926;
                        *o = v * 0.11;
                    }
                }
                NoiseKind::Brown { last } => {
                    // Leaky integration of white noise.
                    for o in out.iter_mut() {
                        *last = (*last + 0.02 * rng.bi()) * 0.998;
                        *o = (*last * 8.0).clamp(-1.0, 1.0);
                    }
                }
            },
            Src::Dust { rng, p, g, y } => {
                for o in out.iter_mut() {
                    let imp = if rng.unit() < *p { rng.bi() } else { 0.0 };
                    *y = imp + *g * *y;
                    *o = *y;
                }
            }
            Src::Seq { buf } => {
                for (i, o) in out.iter_mut().enumerate() {
                    *o = buf.get(t0 + i).copied().unwrap_or(0.0);
                }
            }
            Src::Mix { inputs, scratch } => {
                out.fill(0.0);
                let len = out.len();
                for c in inputs.iter_mut() {
                    let s = &mut scratch[..len];
                    c.process(s, t0, pitch);
                    for (o, v) in out.iter_mut().zip(s.iter()) {
                        *o += *v;
                    }
                }
            }
            Src::Mul { inputs, scratch } => {
                out.fill(1.0);
                let len = out.len();
                for c in inputs.iter_mut() {
                    let s = &mut scratch[..len];
                    c.process(s, t0, pitch);
                    for (o, v) in out.iter_mut().zip(s.iter()) {
                        *o *= *v;
                    }
                }
            }
            Src::Chain { src, procs } => {
                src.process(out, t0, pitch);
                for p in procs.iter_mut() {
                    p.process(out, t0, pitch);
                }
            }
        }
    }

    /// Apply a live cutoff `scale` to every biquad filter in the signal tree —
    /// the instrument brightness control. Recurses through mix/mul/chain.
    pub(super) fn set_cutoff(&mut self, scale: f32) {
        match self {
            Src::Mix { inputs, .. } | Src::Mul { inputs, .. } => {
                inputs.iter_mut().for_each(|c| c.set_cutoff(scale));
            }
            Src::Chain { src, procs } => {
                src.set_cutoff(scale);
                procs.iter_mut().for_each(|p| p.set_cutoff(scale));
            }
            _ => {}
        }
    }
}

/// Build the evaluator of `node` for `ctx`, seeded by its structural `path`.
/// `None` only for a node the context refuses (a SoundFont seq while
/// streaming — [`super::StreamGraph::blockers`] names it); the bounce
/// context builds every node.
pub(super) fn try_src(node: &Node, ctx: &Ctx, path: u64) -> Option<Src> {
    let srf = ctx.srf();
    let n = ctx.n;
    let v = |val: &Value| Val::build(val, ctx.sr, n);
    Some(match node {
        Node::Noise { color } => Src::Noise {
            rng: Rng::new(node_seed(path)),
            kind: match color {
                NoiseColor::White => NoiseKind::White,
                NoiseColor::Pink => NoiseKind::Pink { b: [0.0; 7] },
                NoiseColor::Brown => NoiseKind::Brown { last: 0.0 },
            },
        },
        Node::Dust { density, decay } => {
            let p = (density / srf).clamp(0.0, 1.0);
            let g = if *decay > 0.0 {
                crate::dsp::one_pole(*decay, srf)
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
        // The SoundFont sampler is an external stateful voice: bounce-only.
        Node::Seq { wave, .. } if *wave != SeqWave::Sampler || ctx.offline => Src::Seq {
            buf: seq_to_signal(node, n, ctx.sr, &mut Rng::new(node_seed(path))),
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
                // Decorrelated voice phases; symmetric detune across
                // [-detune, +detune] cents.
                phases.push(k as f32 / count as f32);
                let cents = if count == 1 {
                    0.0
                } else {
                    -detune_cents + 2.0 * detune_cents * (k as f32 / (count as f32 - 1.0))
                };
                ratios.push(crate::dsp::powf(2.0, cents / 1200.0));
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
        Node::Wavetable {
            wave,
            freq,
            position,
        } => Src::Wavetable {
            frames: wavetable_frames(*wave),
            phase: 0.0,
            freq: v(freq),
            position: v(position),
            srf,
        },
        Node::Impact { hardness, velocity } => {
            let h = hardness.clamp(0.0, 1.0);
            let vel = velocity.clamp(0.0, 1.0);
            // Contact time: soft ≈ 8 ms, hard ≈ 0.3 ms. The Hann lobe sums to
            // ≈ w/2, so `norm` gives the whole pulse unit area × velocity.
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
        Node::Mix { inputs } => Src::Mix {
            inputs: inputs
                .iter()
                .enumerate()
                .map(|(i, c)| try_src(c, ctx, node_path(path, i)))
                .collect::<Option<_>>()?,
            scratch: vec![0.0; ctx.cap],
        },
        Node::Mul { inputs } => Src::Mul {
            inputs: inputs
                .iter()
                .enumerate()
                .map(|(i, c)| try_src(c, ctx, node_path(path, i)))
                .collect::<Option<_>>()?,
            scratch: vec![0.0; ctx.cap],
        },
        Node::Chain { stages } => {
            // The last source or combinator stage is the signal and the
            // processors after it transform it; anything before it is
            // discarded (seeds are structural, so nothing leaks). A chain
            // without a source runs its processors on silence.
            let k = stages.iter().rposition(|s| !s.is_processor());
            let (src, first_proc) = match k {
                Some(k) => (try_src(&stages[k], ctx, node_path(path, k))?, k + 1),
                None => (Src::Silence, 0),
            };
            let procs = stages[first_proc..]
                .iter()
                .enumerate()
                .map(|(i, p)| try_proc(p, ctx, node_path(path, first_proc + i)))
                .collect::<Option<_>>()?;
            Src::Chain {
                src: Box::new(src),
                procs,
            }
        }
        // Validation rejects a nested mixer; evaluate one defensively as the
        // plain gain-weighted sum of its tracks.
        Node::Tracks { tracks, .. } => Src::Mix {
            inputs: tracks
                .iter()
                .enumerate()
                .map(|(i, t)| {
                    Some(Src::Chain {
                        src: Box::new(try_src(&t.node, ctx, node_path(path, i))?),
                        procs: vec![Proc::Gain(Val::Const(t.gain))],
                    })
                })
                .collect::<Option<_>>()?,
            scratch: vec![0.0; ctx.cap],
        },
        // A processor standing where a source belongs has no input: silence.
        _ if node.is_processor() => Src::Silence,
        _ => return None,
    })
}
