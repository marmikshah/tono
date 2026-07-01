//! streaming — a stateful, block-by-block renderer for the causal subset of the
//! graph.
//!
//! It carries each node's per-sample state (oscillator phase, filter z-state,
//! modulator walk) across [`fill`](StreamGraph::fill) calls and reuses the
//! offline renderer's exact per-sample math, so a streamed render is
//! **byte-identical to the offline graph evaluation by construction** — and
//! independent of the block size it is pulled in (chunking a deterministic
//! per-sample loop can't change its output). Modulated parameters are supported:
//! Slide/Lfo/Arp/EnvMod are closed-form functions of the absolute sample index,
//! and Rand carries its own self-seeded walk.
//!
//! Graphs outside the subset — RNG leaves (noise, dust, seq) which draw from the
//! shared, evaluation-order-dependent render stream, delay-line effects not yet
//! ported, a `tracks` root (the stereo mixer + master path), or a `normalize`
//! output stage — are rejected by [`StreamGraph::try_from_doc`], and the caller
//! falls back to the buffer-backed [`crate::stream::Player`] (itself
//! byte-identical). Hybrid Phase-1: stream what's provably causal, buffer the rest.

use std::f32::consts::TAU;

use crate::dsl::{Curve, Modulator, Node, Shape, SoundDoc, SuperWave, Value, note_to_hz};
use crate::dsp::Rng;
use crate::render::{osc, poly_blep, rand_seed};

/// The ADSR envelope value at time `t` seconds — the exact body of the offline
/// `adsr` (also used by `Modulator::EnvMod`). `rel_start` anchors the release to
/// the end of the render (`total_secs - r`).
fn adsr_env(t: f32, a: f32, d: f32, s: f32, r: f32, punch: f32, rel_start: f32) -> f32 {
    let mut v = if t < a {
        if a > 0.0 { t / a } else { 1.0 }
    } else if t < a + d {
        let p = if d > 0.0 { (t - a) / d } else { 1.0 };
        1.0 - (1.0 - s) * p
    } else if t < rel_start {
        s
    } else if r > 0.0 {
        let p = ((t - rel_start) / r).clamp(0.0, 1.0);
        s * (1.0 - p)
    } else {
        0.0
    };
    let punch_win = a + d;
    if punch > 0.0 && punch_win > 0.0 && t < punch_win {
        v *= 1.0 + punch * (1.0 - t / punch_win);
    }
    v
}

/// A per-sample evaluator for a dsl [`Value`], byte-identical to `eval_value` at
/// the given absolute sample index. Const/Note are constant; the modulators match
/// the offline formulas. `Rand` is stateful (carries its self-seeded walk) so it
/// must be stepped once per sample in order.
enum Val {
    Const(f32),
    Slide {
        from: f32,
        to: f32,
        secs: f32,
        curve: Curve,
        srf: f32,
    },
    Lfo {
        shape: Shape,
        rate: f32,
        depth: f32,
        center: f32,
        srf: f32,
    },
    Arp {
        steps: Vec<f32>,
        rate: f32,
        srf: f32,
    },
    EnvMod {
        a: f32,
        d: f32,
        s: f32,
        r: f32,
        punch: f32,
        from: f32,
        to: f32,
        srf: f32,
        rel_start: f32,
    },
    Rand {
        rng: Rng,
        prev: f32,
        next: f32,
        phase: f32,
        inc: f32,
        from: f32,
        to: f32,
    },
}

impl Val {
    fn build(v: &Value, sr: u32, n: usize) -> Self {
        let srf = sr as f32;
        match v {
            Value::Const(c) => Val::Const(*c),
            Value::Note(name) => Val::Const(note_to_hz(name).unwrap_or(440.0)),
            Value::Modulated(m) => match m {
                Modulator::Slide {
                    from,
                    to,
                    secs,
                    curve,
                } => Val::Slide {
                    from: *from,
                    to: *to,
                    secs: *secs,
                    curve: *curve,
                    srf,
                },
                Modulator::Lfo {
                    shape,
                    rate,
                    depth,
                    center,
                } => Val::Lfo {
                    shape: *shape,
                    rate: *rate,
                    depth: *depth,
                    center: *center,
                    srf,
                },
                Modulator::Arp { steps, rate } => Val::Arp {
                    steps: steps.clone(),
                    rate: *rate,
                    srf,
                },
                Modulator::EnvMod { adsr, from, to } => Val::EnvMod {
                    a: adsr.a,
                    d: adsr.d,
                    s: adsr.s,
                    r: adsr.r,
                    punch: adsr.punch,
                    from: *from,
                    to: *to,
                    srf,
                    rel_start: (n as f32 / srf - adsr.r).max(0.0),
                },
                Modulator::Rand {
                    from,
                    to,
                    rate,
                    seed,
                } => {
                    let mut rng = Rng::new(rand_seed(*seed, *from, *to, *rate));
                    let inc = rate.max(1e-4) / srf;
                    let prev = rng.range(*from, *to);
                    let next = rng.range(*from, *to);
                    Val::Rand {
                        rng,
                        prev,
                        next,
                        phase: 0.0,
                        inc,
                        from: *from,
                        to: *to,
                    }
                }
            },
        }
    }

    fn eval(&mut self, t: usize) -> f32 {
        match self {
            Val::Const(c) => *c,
            Val::Slide {
                from,
                to,
                secs,
                curve,
                srf,
            } => {
                let tt = t as f32 / *srf;
                let p = (tt / *secs).clamp(0.0, 1.0);
                match curve {
                    Curve::Lin => *from + (*to - *from) * p,
                    Curve::Exp if *from > 0.0 && *to > 0.0 => *from * (*to / *from).powf(p),
                    Curve::Exp => {
                        let e = p * p;
                        *from + (*to - *from) * e
                    }
                }
            }
            Val::Lfo {
                shape,
                rate,
                depth,
                center,
                srf,
            } => {
                let phase = (t as f32 / *srf * *rate).fract();
                *center + *depth * osc(*shape, phase)
            }
            Val::Arp { steps, rate, srf } => {
                let tt = t as f32 / *srf;
                let idx = (tt * *rate) as usize % steps.len();
                steps[idx]
            }
            Val::EnvMod {
                a,
                d,
                s,
                r,
                punch,
                from,
                to,
                srf,
                rel_start,
            } => {
                let v = adsr_env(t as f32 / *srf, *a, *d, *s, *r, *punch, *rel_start);
                *from + (*to - *from) * v
            }
            Val::Rand {
                rng,
                prev,
                next,
                phase,
                inc,
                from,
                to,
            } => {
                let sm = *phase * *phase * (3.0 - 2.0 * *phase);
                let out = *prev + (*next - *prev) * sm;
                *phase += *inc;
                while *phase >= 1.0 {
                    *phase -= 1.0;
                    *prev = *next;
                    *next = rng.range(*from, *to);
                }
                out
            }
        }
    }
}

/// A streamable source / combinator node, holding its per-sample state.
enum Src {
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
    Mix(Vec<Src>),
    Mul(Vec<Src>),
    Chain {
        src: Box<Src>,
        procs: Vec<Proc>,
    },
}

/// A streamable processor, holding its per-sample state.
enum Proc {
    Gain(f32),
    Biquad {
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
}

impl Src {
    /// The next sample at absolute index `t`. Mirrors the offline per-sample math.
    fn step(&mut self, t: usize) -> f32 {
        match self {
            Src::Sine { phase, freq, srf } => {
                let v = osc(Shape::Sine, *phase);
                *phase += freq.eval(t).max(0.0) / *srf;
                *phase -= phase.floor();
                v
            }
            Src::Square {
                phase,
                freq,
                duty,
                srf,
            } => {
                let dt = freq.eval(t).max(0.0) / *srf;
                let du = duty.eval(t).clamp(0.01, 0.99);
                let mut v = if *phase < du { 1.0 } else { -1.0 };
                v += poly_blep(*phase, dt);
                v -= poly_blep((*phase - du + 1.0).fract(), dt);
                *phase += dt;
                *phase -= phase.floor();
                v
            }
            Src::Saw { phase, freq, srf } => {
                let dt = freq.eval(t).max(0.0) / *srf;
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
                let dt = freq.eval(t).max(0.0) / *srf;
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
                let fi = freq.eval(t).max(0.0);
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
                let f = freq.eval(t).max(0.0);
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
            Src::Mix(cs) => {
                let mut acc = 0.0f32;
                for c in cs.iter_mut() {
                    acc += c.step(t);
                }
                acc
            }
            Src::Mul(cs) => {
                let mut acc = 1.0f32;
                for c in cs.iter_mut() {
                    acc *= c.step(t);
                }
                acc
            }
            Src::Chain { src, procs } => {
                let mut x = src.step(t);
                for p in procs.iter_mut() {
                    x = p.step(x);
                }
                x
            }
        }
    }
}

impl Proc {
    fn step(&mut self, x0: f32) -> f32 {
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
            } => {
                let y0 = *b0 * x0 + *b1 * *x1 + *b2 * *x2 - *a1 * *y1 - *a2 * *y2;
                *x2 = *x1;
                *x1 = x0;
                *y2 = *y1;
                *y1 = y0;
                y0
            }
        }
    }
}

/// The RBJ biquad kinds we stream (constant cutoff), each with its shelf/peak gain.
enum Filt {
    Low,
    High,
    Band,
    Notch,
    Peak(f32),
    LowShelf(f32),
    HighShelf(f32),
}

/// Precompute the (a0-normalised) biquad coefficients — identical values to the
/// offline `biquad`, which recomputes them per sample from a constant cutoff.
fn biquad(kind: Filt, fc: f32, q: f32, sr: u32) -> Proc {
    let srf = sr as f32;
    let q = q.max(0.05);
    let nyq = srf / 2.0;
    let f = fc.clamp(20.0, nyq - 100.0);
    let w0 = TAU * f / srf;
    let (sin, cos) = w0.sin_cos();
    let alpha = sin / (2.0 * q);
    let amp = match kind {
        Filt::Peak(g) | Filt::LowShelf(g) | Filt::HighShelf(g) => 10f32.powf(g / 40.0),
        _ => 1.0,
    };
    let (b0, b1, b2, a0, a1, a2) = match kind {
        Filt::Low => (
            (1.0 - cos) / 2.0,
            1.0 - cos,
            (1.0 - cos) / 2.0,
            1.0 + alpha,
            -2.0 * cos,
            1.0 - alpha,
        ),
        Filt::High => (
            (1.0 + cos) / 2.0,
            -(1.0 + cos),
            (1.0 + cos) / 2.0,
            1.0 + alpha,
            -2.0 * cos,
            1.0 - alpha,
        ),
        Filt::Band => (alpha, 0.0, -alpha, 1.0 + alpha, -2.0 * cos, 1.0 - alpha),
        Filt::Notch => (1.0, -2.0 * cos, 1.0, 1.0 + alpha, -2.0 * cos, 1.0 - alpha),
        Filt::Peak(_) => (
            1.0 + alpha * amp,
            -2.0 * cos,
            1.0 - alpha * amp,
            1.0 + alpha / amp,
            -2.0 * cos,
            1.0 - alpha / amp,
        ),
        Filt::LowShelf(_) => {
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
        Filt::HighShelf(_) => {
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
    Proc::Biquad {
        b0: b0 / a0,
        b1: b1 / a0,
        b2: b2 / a0,
        a1: a1 / a0,
        a2: a2 / a0,
        x1: 0.0,
        x2: 0.0,
        y1: 0.0,
        y2: 0.0,
    }
}

fn try_src(node: &Node, sr: u32, n: usize) -> Option<Src> {
    let srf = sr as f32;
    let v = |val: &Value| Val::build(val, sr, n);
    Some(match node {
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
                .map(|i| try_src(i, sr, n))
                .collect::<Option<_>>()?,
        ),
        Node::Mul { inputs } => Src::Mul(
            inputs
                .iter()
                .map(|i| try_src(i, sr, n))
                .collect::<Option<_>>()?,
        ),
        Node::Chain { stages } => {
            let (first, rest) = stages.split_first()?;
            let src = Box::new(try_src(first, sr, n)?);
            let procs = rest
                .iter()
                .map(|p| try_proc(p, sr))
                .collect::<Option<_>>()?;
            Src::Chain { src, procs }
        }
        _ => return None,
    })
}

fn try_proc(node: &Node, sr: u32) -> Option<Proc> {
    // Filters/EQ only stream with a constant cutoff.
    let cst = |val: &Value| match val {
        Value::Const(c) => Some(*c),
        Value::Note(s) => Some(note_to_hz(s).unwrap_or(440.0)),
        Value::Modulated(_) => None,
    };
    Some(match node {
        Node::Gain { amount } => Proc::Gain(cst(amount)?),
        Node::Lowpass { cutoff, q } => biquad(Filt::Low, cst(cutoff)?, *q, sr),
        Node::Highpass { cutoff, q } => biquad(Filt::High, cst(cutoff)?, *q, sr),
        Node::Bandpass { cutoff, q } => biquad(Filt::Band, cst(cutoff)?, *q, sr),
        Node::Notch { cutoff, q } => biquad(Filt::Notch, cst(cutoff)?, *q, sr),
        Node::Peak { cutoff, q, gain_db } => biquad(Filt::Peak(*gain_db), cst(cutoff)?, *q, sr),
        Node::Lowshelf { cutoff, gain_db } => {
            biquad(Filt::LowShelf(*gain_db), cst(cutoff)?, 0.707, sr)
        }
        Node::Highshelf { cutoff, gain_db } => {
            biquad(Filt::HighShelf(*gain_db), cst(cutoff)?, 0.707, sr)
        }
        _ => return None,
    })
}

/// A stateful, block-by-block renderer for a supported graph.
pub struct StreamGraph {
    root: Src,
    pos: usize,
}

impl StreamGraph {
    /// Build a streamer for `doc`, or `None` if the graph is outside the
    /// streamable subset — the caller then falls back to the buffer-backed
    /// [`crate::stream::Player`].
    pub fn try_from_doc(doc: &SoundDoc) -> Option<Self> {
        if doc.normalize.is_some() || matches!(doc.root, Node::Tracks { .. }) {
            return None;
        }
        let n = ((doc.duration * doc.sample_rate as f32).ceil() as usize).max(1);
        Some(StreamGraph {
            root: try_src(&doc.root, doc.sample_rate, n)?,
            pos: 0,
        })
    }

    /// Fill `out` with the next block of mono samples, advancing graph state.
    pub fn fill(&mut self, out: &mut [f32]) {
        for s in out.iter_mut() {
            *s = self.root.step(self.pos);
            self.pos += 1;
        }
    }
}

/// Whether `doc`'s graph can be streamed. A cheap check the runtime uses to pick
/// the streaming path.
pub fn is_streamable(doc: &SoundDoc) -> bool {
    StreamGraph::try_from_doc(doc).is_some()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::render::render_graph;

    fn bits(s: &[f32]) -> Vec<u32> {
        s.iter().map(|x| x.to_bits()).collect()
    }

    fn parse(json: &str) -> SoundDoc {
        serde_json::from_str(json).unwrap()
    }

    /// Assert a doc streams byte-for-byte identical to the offline graph, in one
    /// block and split across several block sizes.
    fn assert_byte_identical(doc: &SoundDoc) {
        let offline = render_graph(doc);
        let mut sg = StreamGraph::try_from_doc(doc).expect("should be streamable");
        let mut whole = vec![0.0f32; offline.len()];
        sg.fill(&mut whole);
        assert_eq!(
            bits(&whole),
            bits(&offline),
            "whole-block stream != offline"
        );
        for bs in [1usize, 7, 64, 333] {
            let mut sg = StreamGraph::try_from_doc(doc).unwrap();
            let mut got: Vec<f32> = Vec::with_capacity(offline.len());
            while got.len() < offline.len() {
                let take = bs.min(offline.len() - got.len());
                let mut blk = vec![0.0f32; take];
                sg.fill(&mut blk);
                got.extend(blk);
            }
            assert_eq!(bits(&got), bits(&offline), "block size {bs} != offline");
        }
    }

    #[test]
    fn filtered_square() {
        assert_byte_identical(&parse(
            r#"{ "name":"s", "duration":0.1, "root": { "type":"chain", "stages": [
                { "type":"square", "freq":220 },
                { "type":"lowpass", "cutoff":800, "q":0.7 } ] } }"#,
        ));
    }

    #[test]
    fn mix_of_oscillators() {
        assert_byte_identical(&parse(
            r#"{ "name":"m", "duration":0.05, "root": { "type":"mix", "inputs": [
                { "type":"sine", "freq":440 },
                { "type":"sawtooth", "freq":110 } ] } }"#,
        ));
    }

    #[test]
    fn lfo_modulated_frequency() {
        assert_byte_identical(&parse(
            r#"{ "name":"l", "duration":0.08, "root":
                { "type":"sine", "freq": { "lfo": { "shape":"sine", "rate":6, "depth":80, "center":440 } } } }"#,
        ));
    }

    #[test]
    fn slide_and_arp_modulators() {
        assert_byte_identical(&parse(
            r#"{ "name":"sl", "duration":0.1, "root":
                { "type":"sawtooth", "freq": { "slide": { "from":110, "to":880, "secs":0.09, "curve":"lin" } } } }"#,
        ));
        assert_byte_identical(&parse(
            r#"{ "name":"ar", "duration":0.1, "root":
                { "type":"square", "freq": { "arp": { "steps":[220,330,440], "rate":20 } } } }"#,
        ));
    }

    #[test]
    fn rand_modulator_carries_its_walk() {
        assert_byte_identical(&parse(
            r#"{ "name":"rn", "duration":0.1, "root":
                { "type":"sine", "freq": { "rand": { "from":200, "to":600, "rate":15, "seed":42 } } } }"#,
        ));
    }

    #[test]
    fn fm_and_super_sources() {
        assert_byte_identical(&parse(
            r#"{ "name":"fm", "duration":0.05, "root": { "type":"fm", "freq":220, "ratio":2.0, "index":5.0 } }"#,
        ));
        assert_byte_identical(&parse(
            r#"{ "name":"su", "duration":0.05, "root":
                { "type":"super", "wave":"sawtooth", "freq":110, "voices":7, "detune_cents":18 } }"#,
        ));
    }

    #[test]
    fn impact_and_env() {
        assert_byte_identical(&parse(
            r#"{ "name":"im", "duration":0.05, "root": { "type":"impact", "hardness":0.6, "velocity":0.9 } }"#,
        ));
        assert_byte_identical(&parse(
            r#"{ "name":"ev", "duration":0.2, "root": { "type":"mul", "inputs": [
                { "type":"sine", "freq":330 },
                { "type":"env", "adsr": { "a":0.01, "d":0.05, "s":0.4, "r":0.1 } } ] } }"#,
        ));
    }

    #[test]
    fn peak_and_shelf_eq() {
        assert_byte_identical(&parse(
            r#"{ "name":"eq", "duration":0.06, "root": { "type":"chain", "stages": [
                { "type":"sawtooth", "freq":150 },
                { "type":"peak", "cutoff":1200, "q":1.5, "gain_db":6 },
                { "type":"lowshelf", "cutoff":200, "gain_db":-4 },
                { "type":"highshelf", "cutoff":4000, "gain_db":3 } ] } }"#,
        ));
    }

    #[test]
    fn non_streamable_graphs_are_rejected() {
        assert!(
            StreamGraph::try_from_doc(&parse(
                r#"{ "name":"n", "duration":0.05, "root": { "type":"noise", "color":"white" } }"#
            ))
            .is_none()
        );
        assert!(
            StreamGraph::try_from_doc(&parse(
                r#"{ "name":"t", "duration":0.05, "root": { "type":"tracks", "tracks": [
                    { "node": { "type":"sine", "freq":440 } } ] } }"#
            ))
            .is_none()
        );
    }
}
