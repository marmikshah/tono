//! streaming — a stateful, block-by-block renderer for the causal, constant-
//! parameter, RNG-free subset of the graph.
//!
//! It carries each node's per-sample state (oscillator phase, filter z-state)
//! across [`fill`](StreamGraph::fill) calls and reuses the offline renderer's
//! exact per-sample math, so a streamed render is **byte-identical to the offline
//! graph evaluation by construction** — and independent of the block size it is
//! pulled in (chunking a deterministic per-sample loop can't change its output).
//!
//! Graphs outside the subset — modulated (time-varying) parameters, RNG or
//! whole-buffer nodes (noise, seq, reverb, delay, …), a `tracks` root, or a
//! `normalize` output stage — are rejected by [`StreamGraph::try_from_doc`], and
//! the caller falls back to the buffer-backed [`crate::stream::Player`] (which is
//! itself byte-identical). This is the hybrid Phase-1 renderer: stream what is
//! provably causal, buffer the rest.

use std::f32::consts::TAU;

use crate::dsl::{Node, Shape, SoundDoc, Value, note_to_hz};
use crate::render::{osc, poly_blep};

/// A constant scalar from a [`Value`] — `None` for a modulated (time-varying)
/// value, which is not (yet) streamable.
fn constant(v: &Value) -> Option<f32> {
    match v {
        Value::Const(c) => Some(*c),
        Value::Note(s) => Some(note_to_hz(s).unwrap_or(440.0)),
        Value::Modulated(_) => None,
    }
}

/// A streamable source / combinator node, holding its per-sample state.
enum Src {
    Sine { phase: f32, dt: f32 },
    Square { phase: f32, dt: f32, duty: f32 },
    Saw { phase: f32, dt: f32 },
    Tri { phase: f32, dt: f32, tri: f32 },
    Mix(Vec<Src>),
    Mul(Vec<Src>),
    Chain { src: Box<Src>, procs: Vec<Proc> },
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
    /// The next sample. Mirrors the offline per-sample math exactly.
    fn step(&mut self) -> f32 {
        match self {
            Src::Sine { phase, dt } => {
                let v = osc(Shape::Sine, *phase);
                *phase += *dt;
                *phase -= phase.floor();
                v
            }
            Src::Square { phase, dt, duty } => {
                let mut v = if *phase < *duty { 1.0 } else { -1.0 };
                v += poly_blep(*phase, *dt);
                v -= poly_blep((*phase - *duty + 1.0).fract(), *dt);
                *phase += *dt;
                *phase -= phase.floor();
                v
            }
            Src::Saw { phase, dt } => {
                let v = (2.0 * *phase - 1.0) - poly_blep(*phase, *dt);
                *phase += *dt;
                *phase -= phase.floor();
                v
            }
            Src::Tri { phase, dt, tri } => {
                let mut sq = if *phase < 0.5 { 1.0 } else { -1.0 };
                sq += poly_blep(*phase, *dt);
                sq -= poly_blep((*phase + 0.5).fract(), *dt);
                *tri = *tri * 0.9995 + 4.0 * *dt * sq;
                let v = *tri;
                *phase += *dt;
                *phase -= phase.floor();
                v
            }
            Src::Mix(cs) => {
                let mut acc = 0.0f32;
                for c in cs.iter_mut() {
                    acc += c.step();
                }
                acc
            }
            Src::Mul(cs) => {
                let mut acc = 1.0f32;
                for c in cs.iter_mut() {
                    acc *= c.step();
                }
                acc
            }
            Src::Chain { src, procs } => {
                let mut x = src.step();
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

/// The RBJ biquad kinds we stream (constant cutoff / q).
enum Filt {
    Low,
    High,
    Band,
    Notch,
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

fn try_src(node: &Node, sr: u32) -> Option<Src> {
    let srf = sr as f32;
    Some(match node {
        Node::Sine { freq } => Src::Sine {
            phase: 0.0,
            dt: constant(freq)?.max(0.0) / srf,
        },
        Node::Square { freq, duty } => Src::Square {
            phase: 0.0,
            dt: constant(freq)?.max(0.0) / srf,
            duty: constant(duty)?.clamp(0.01, 0.99),
        },
        Node::Sawtooth { freq } => Src::Saw {
            phase: 0.0,
            dt: constant(freq)?.max(0.0) / srf,
        },
        Node::Triangle { freq } => Src::Tri {
            phase: 0.0,
            dt: constant(freq)?.max(0.0) / srf,
            tri: 0.0,
        },
        Node::Mix { inputs } => Src::Mix(
            inputs
                .iter()
                .map(|i| try_src(i, sr))
                .collect::<Option<_>>()?,
        ),
        Node::Mul { inputs } => Src::Mul(
            inputs
                .iter()
                .map(|i| try_src(i, sr))
                .collect::<Option<_>>()?,
        ),
        Node::Chain { stages } => {
            let (first, rest) = stages.split_first()?;
            let src = Box::new(try_src(first, sr)?);
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
    Some(match node {
        Node::Gain { amount } => Proc::Gain(constant(amount)?),
        Node::Lowpass { cutoff, q } => biquad(Filt::Low, constant(cutoff)?, *q, sr),
        Node::Highpass { cutoff, q } => biquad(Filt::High, constant(cutoff)?, *q, sr),
        Node::Bandpass { cutoff, q } => biquad(Filt::Band, constant(cutoff)?, *q, sr),
        Node::Notch { cutoff, q } => biquad(Filt::Notch, constant(cutoff)?, *q, sr),
        _ => return None,
    })
}

/// A stateful, block-by-block renderer for a supported graph.
pub struct StreamGraph {
    root: Src,
}

impl StreamGraph {
    /// Build a streamer for `doc`, or `None` if the graph is outside the
    /// streamable subset — the caller then falls back to the buffer-backed
    /// [`crate::stream::Player`].
    pub fn try_from_doc(doc: &SoundDoc) -> Option<Self> {
        if doc.normalize.is_some() || matches!(doc.root, Node::Tracks { .. }) {
            return None;
        }
        Some(StreamGraph {
            root: try_src(&doc.root, doc.sample_rate)?,
        })
    }

    /// Fill `out` with the next block of mono samples, advancing graph state.
    pub fn fill(&mut self, out: &mut [f32]) {
        for s in out.iter_mut() {
            *s = self.root.step();
        }
    }
}

/// Whether `doc`'s graph can be streamed (all nodes causal, constant-param, and
/// RNG-free). A cheap check the runtime uses to pick the streaming path.
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

    fn filtered_square() -> SoundDoc {
        parse(
            r#"{ "name":"s", "duration":0.1, "root": { "type":"chain", "stages": [
                { "type":"square", "freq":220 },
                { "type":"lowpass", "cutoff":800, "q":0.7 } ] } }"#,
        )
    }

    #[test]
    fn streamed_matches_offline_graph_byte_for_byte() {
        let d = filtered_square();
        let offline = render_graph(&d);
        let mut sg = StreamGraph::try_from_doc(&d).expect("streamable");
        let mut streamed = vec![0.0f32; offline.len()];
        sg.fill(&mut streamed);
        assert_eq!(bits(&streamed), bits(&offline));
    }

    #[test]
    fn block_size_invariant() {
        let d = filtered_square();
        let offline = render_graph(&d);
        for bs in [1usize, 7, 64, 333] {
            let mut sg = StreamGraph::try_from_doc(&d).unwrap();
            let mut got: Vec<f32> = Vec::with_capacity(offline.len());
            while got.len() < offline.len() {
                let take = bs.min(offline.len() - got.len());
                let mut blk = vec![0.0f32; take];
                sg.fill(&mut blk);
                got.extend(blk);
            }
            assert_eq!(
                bits(&got),
                bits(&offline),
                "block size {bs} must match offline"
            );
        }
    }

    #[test]
    fn mix_of_oscillators_is_byte_identical() {
        let d = parse(
            r#"{ "name":"m", "duration":0.05, "root": { "type":"mix", "inputs": [
                { "type":"sine", "freq":440 },
                { "type":"sawtooth", "freq":110 } ] } }"#,
        );
        let offline = render_graph(&d);
        let mut sg = StreamGraph::try_from_doc(&d).unwrap();
        let mut out = vec![0.0f32; offline.len()];
        sg.fill(&mut out);
        assert_eq!(bits(&out), bits(&offline));
    }

    #[test]
    fn non_streamable_graphs_are_rejected() {
        // Noise carries RNG (eval-order dependent) → not streamable.
        assert!(
            StreamGraph::try_from_doc(&parse(
                r#"{ "name":"n", "duration":0.05, "root": { "type":"noise", "color":"white" } }"#
            ))
            .is_none()
        );
        // A tracks root uses the stereo mixer path.
        assert!(
            StreamGraph::try_from_doc(&parse(
                r#"{ "name":"t", "duration":0.05, "root": { "type":"tracks", "tracks": [
                    { "node": { "type":"sine", "freq":440 } } ] } }"#
            ))
            .is_none()
        );
    }
}
