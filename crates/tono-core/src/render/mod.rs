//! Deterministic graph → samples renderer.
//!
//! Rendering is a pure function of `(graph, seed, sample_rate)`. Each node is
//! evaluated into a block of `f32` samples; combinators combine those blocks.
//! Processors transform the signal flowing through a `chain`.

mod effects;
mod kit;
mod osc;
mod seq;
#[cfg(test)]
mod tests;

pub(crate) use effects::{FilterKind, biquad_coeffs, drive_antideriv, drive_curve};
pub(crate) use osc::{osc, poly_blep};
pub(crate) use seq::seq_to_signal;

use crate::dsl::{Adsr, Curve, Modulator, Node, Playback, Shape, SoundDoc, Value};
use crate::dsp::{Rng, node_path, node_seed, peak_limit};
use effects::{biquad, chorus, compress, drive_adaa, flanger, modal_bank, phaser, reverb};
use osc::{
    dust_signal, fm_signal, impact_signal, noise_signal, osc_signal, saw_signal, square_signal,
    super_signal, tri_signal,
};
use std::f32::consts::TAU;

/// A block of mono audio samples.
type Signal = Vec<f32>;

/// A finished render: the mono mid (what analysis and mono export consume)
/// plus the true stereo bus when the document is a `tracks` mixer. Producing
/// both from ONE render keeps the author/refine/export paths from paying the
/// full synthesis cost twice. Plain documents carry no pair here — their
/// `stereo` treatment (Haas / Wide) is applied at write time by [`stereoize`].
pub struct RenderProduct {
    /// Mono mid signal: `0.5 × (L + R)` for a mixer document, the render
    /// itself otherwise.
    pub mono: Signal,
    /// The panned, mastered stereo bus of a `tracks` document.
    pub stereo: Option<(Signal, Signal)>,
    /// Per-layer contribution stats for a `tracks` document (post-fader,
    /// pre-master), captured from the same render pass.
    pub layers: Vec<LayerStats>,
}

mod output;
mod tracks;

pub use output::{loop_seam_db, make_loop_buffer, stereoize};
use output::{normalize_output, normalize_output_v4};
pub use tracks::{LayerStats, TracksRender, render_tracks};

/// Render a sound document once, yielding the mono mid plus the stereo bus
/// for mixer documents. Every consumer that needs both (analysis + the WAV on
/// disk) should call this instead of rendering twice.
pub fn render_product(doc: &SoundDoc) -> RenderProduct {
    if let Some(tr) = render_tracks(doc) {
        // Mono consumers (analysis, mono export) get the mid signal.
        let mono = tr
            .left
            .iter()
            .zip(&tr.right)
            .map(|(a, b)| 0.5 * (a + b))
            .collect();
        return RenderProduct {
            mono,
            stereo: Some((tr.left, tr.right)),
            layers: tr.layers,
        };
    }
    RenderProduct {
        mono: render_plain(doc),
        stereo: None,
        layers: Vec::new(),
    }
}

/// Render a sound document to normalized mono samples in [-1, 1].
pub fn render(doc: &SoundDoc) -> Signal {
    render_product(doc).mono
}

/// Raw graph evaluation — [`render_node`] on the root with no output stage (loop
/// / normalize / peak-limit / stereo). This is the reference the streaming
/// renderer matches byte-for-byte (used by the streaming byte-identity tests).
#[cfg(test)]
pub(crate) fn render_graph(doc: &SoundDoc) -> Signal {
    let sr = doc.sample_rate;
    let n = ((doc.duration * sr as f32).ceil() as usize).max(1);
    let mut rng = Rng::new(doc.seed);
    render_node(&doc.root, n, sr, &mut rng, doc.effective_engine(), doc.seed)
}

/// The non-mixer render path: one graph, one mono buffer.
fn render_plain(doc: &SoundDoc) -> Signal {
    let sr = doc.sample_rate;
    let n = ((doc.duration * sr as f32).ceil() as usize).max(1);
    let mut rng = Rng::new(doc.seed);
    let engine = doc.effective_engine();
    let mut out = render_node(&doc.root, n, sr, &mut rng, engine, doc.seed);
    // A loop is rendered as its seamless body (tail crossfaded onto the head).
    if let Playback::Loop {
        start_secs,
        end_secs,
        crossfade_secs,
    } = doc.playback
    {
        out = make_loop_buffer(&out, sr, start_secs, end_secs, crossfade_secs);
    }
    match &doc.normalize {
        // Loudness-matched / true-peak-limited output stage (opt-in).
        Some(nz) if engine >= 4 => normalize_output_v4(&mut [&mut out], nz, sr),
        Some(nz) => normalize_output(&mut out, nz),
        // Default: a transparent sample-peak safety limit only.
        None => peak_limit(&mut [&mut out]),
    }
    out
}

/// Evaluate a parameter into a per-sample buffer of length `n`.
fn eval_value(v: &Value, n: usize, sr: u32) -> Vec<f32> {
    let srf = sr as f32;
    match v {
        Value::Const(c) => vec![*c; n],
        Value::Note(s) => vec![crate::dsl::note_to_hz(s).unwrap_or(440.0); n],
        Value::Modulated(Modulator::Slide {
            from,
            to,
            secs,
            curve,
        }) => (0..n)
            .map(|i| {
                let t = i as f32 / srf;
                // Floor `secs` so an unvalidated `secs == 0` can't make `t/secs`
                // a NaN that then poisons the whole render.
                let p = (t / secs.max(1e-6)).clamp(0.0, 1.0);
                match curve {
                    Curve::Lin => from + (to - from) * p,
                    Curve::Exp if *from > 0.0 && *to > 0.0 => {
                        // Geometric interpolation (natural for pitch / cutoff).
                        from * (to / from).powf(p)
                    }
                    Curve::Exp => {
                        // Fall back to an eased curve when values cross zero.
                        let e = p * p;
                        from + (to - from) * e
                    }
                }
            })
            .collect(),
        Value::Modulated(Modulator::Lfo {
            shape,
            rate,
            depth,
            center,
        }) => (0..n)
            .map(|i| {
                let phase = (i as f32 / srf * rate).fract();
                center + depth * osc(*shape, phase)
            })
            .collect(),
        Value::Modulated(Modulator::Arp { steps, rate }) if !steps.is_empty() => (0..n)
            .map(|i| {
                let t = i as f32 / srf;
                let idx = (t * rate) as usize % steps.len();
                steps[idx]
            })
            .collect(),
        // Empty steps would divide by zero; an unvalidated doc must not panic.
        Value::Modulated(Modulator::Arp { .. }) => vec![0.0; n],
        Value::Modulated(Modulator::EnvMod {
            adsr: env,
            from,
            to,
        }) => {
            let e = adsr(env, n, sr);
            e.iter().map(|x| from + (to - from) * x).collect()
        }
        Value::Modulated(Modulator::Rand {
            from,
            to,
            rate,
            seed,
        }) => {
            // Smoothstep-interpolated random walk between `from` and `to`,
            // drawing a fresh target every 1/`rate` seconds. Seeded ONLY from
            // this modulator's own fields, so it is deterministic and stable
            // under sibling edits (it never touches the shared render stream).
            let mut rng = Rng::new(rand_seed(*seed, *from, *to, *rate));
            let inc = rate.max(1e-4) / srf; // segments per sample
            let (mut prev, mut next) = (rng.range(*from, *to), rng.range(*from, *to));
            let mut phase = 0.0f32;
            (0..n)
                .map(|_| {
                    // Smoothstep for organic, slope-continuous motion.
                    let s = phase * phase * (3.0 - 2.0 * phase);
                    let v = prev + (next - prev) * s;
                    phase += inc;
                    while phase >= 1.0 {
                        phase -= 1.0;
                        prev = next;
                        next = rng.range(*from, *to);
                    }
                    v
                })
                .collect()
        }
    }
}

/// Edit-stable seed for a [`Modulator::Rand`]: a hash of only the modulator's
/// own fields, so its walk never shifts when sibling nodes are added or
/// removed (the random stream is not threaded through graph traversal).
pub(crate) fn rand_seed(seed: u64, from: f32, to: f32, rate: f32) -> u64 {
    let mut h = seed ^ crate::dsp::GOLDEN_GAMMA;
    for bits in [from.to_bits(), to.to_bits(), rate.to_bits()] {
        h = (h ^ bits as u64).wrapping_mul(crate::dsp::FNV_PRIME);
    }
    h
}

/// Render a node into a signal of length `n`. `engine` is the document's
/// DSP-kernel revision (see [`crate::dsl::ENGINE_VERSION`]); kernels that
/// changed output across revisions branch on it so older documents stay
/// byte-identical.
fn render_node(node: &Node, n: usize, sr: u32, rng: &mut Rng, engine: u32, path: u64) -> Signal {
    match node {
        Node::Square { freq, duty } => square_signal(freq, duty, n, sr),
        Node::Triangle { freq } => tri_signal(freq, n, sr),
        Node::Sawtooth { freq } => saw_signal(freq, n, sr),
        Node::Super {
            wave,
            freq,
            voices,
            detune_cents,
        } => super_signal(*wave, freq, *voices, *detune_cents, n, sr),
        Node::Sine { freq } => osc_signal(freq, n, sr, |p| osc(Shape::Sine, p)),
        Node::Noise { color } => {
            // Engine ≥ 2: each noise leaf owns a structurally-seeded stream (from
            // its graph position), so its randomness is independent of traversal
            // order and reproduces byte-identically in the streaming renderer.
            if engine >= 2 {
                let mut local = Rng::new(node_seed(path));
                noise_signal(*color, n, &mut local)
            } else {
                noise_signal(*color, n, rng)
            }
        }
        Node::Fm { freq, ratio, index } => fm_signal(freq, *ratio, index, n, sr),
        // Engine ≥ 2: the seq draws its voice randomness (noise/pluck/kit/thump)
        // from a structurally-seeded stream, so it's order-independent and the
        // streaming renderer reproduces it byte-identically.
        Node::Seq { .. } => {
            if engine >= 2 {
                let mut local = Rng::new(node_seed(path));
                seq_to_signal(node, n, sr, &mut local, engine)
            } else {
                seq_to_signal(node, n, sr, rng, engine)
            }
        }
        Node::Impact { hardness, velocity } => impact_signal(*hardness, *velocity, n, sr),
        Node::Dust { density, decay } => {
            if engine >= 2 {
                let mut local = Rng::new(node_seed(path));
                dust_signal(*density, *decay, n, sr, &mut local)
            } else {
                dust_signal(*density, *decay, n, sr, rng)
            }
        }
        Node::Env { adsr: env } => adsr(env, n, sr),
        // Validation rejects nested mixers; render defensively as a plain sum.
        Node::Tracks { tracks, .. } => {
            let mut acc = vec![0.0f32; n];
            for (i, t) in tracks.iter().enumerate() {
                let sig = render_node(&t.node, n, sr, rng, engine, node_path(path, i));
                for (o, v) in acc.iter_mut().zip(sig) {
                    *o += v * t.gain;
                }
            }
            acc
        }
        Node::Mix { inputs } => {
            let mut acc = vec![0.0f32; n];
            for (i, input) in inputs.iter().enumerate() {
                let s = render_node(input, n, sr, rng, engine, node_path(path, i));
                for (o, v) in acc.iter_mut().zip(s) {
                    *o += v;
                }
            }
            acc
        }
        Node::Mul { inputs } => {
            let mut acc = vec![1.0f32; n];
            for (i, input) in inputs.iter().enumerate() {
                let s = render_node(input, n, sr, rng, engine, node_path(path, i));
                for (o, v) in acc.iter_mut().zip(s) {
                    *o *= v;
                }
            }
            acc
        }
        Node::Chain { stages } => {
            let mut buf: Option<Signal> = None;
            for (i, stage) in stages.iter().enumerate() {
                let cp = node_path(path, i);
                buf = Some(match (&buf, stage.is_processor()) {
                    // A processor transforms the running signal.
                    (Some(input), true) => apply_processor(stage, input, sr, rng, engine, cp),
                    // A source/combinator as a later stage replaces the signal.
                    (_, _) => render_node(stage, n, sr, rng, engine, cp),
                });
            }
            buf.unwrap_or_else(|| vec![0.0; n])
        }
        // A processor rendered standalone (outside a chain) has no input ⇒ silence.
        _ if node.is_processor() => vec![0.0; n],
        // Every non-processor variant is matched above; this fires only if a
        // new source is added to the DSL without a render arm.
        _ => unreachable!("unhandled source node in render_node"),
    }
}

/// Apply a processor node to an incoming signal. (`rng` feeds processors that
/// render an internal side signal, e.g. `duck`'s trigger.) `engine` is the
/// document's DSP-kernel revision; quality-changing processors branch on it so
/// older documents stay byte-identical.
fn apply_processor(
    node: &Node,
    input: &[f32],
    sr: u32,
    rng: &mut Rng,
    engine: u32,
    path: u64,
) -> Signal {
    match node {
        Node::Duck {
            trigger,
            amount,
            attack,
            release,
        } => {
            // Render the trigger silently; its loudness envelope steers a
            // gain dip on the chained signal — the sidechain pump.
            let trig = render_node(trigger, input.len(), sr, rng, engine, node_path(path, 0));
            let srf = sr as f32;
            let at = (-1.0 / (attack.max(1e-4) * srf)).exp();
            let rt = (-1.0 / (release.max(1e-4) * srf)).exp();
            let mut env = 0.0f32;
            input
                .iter()
                .zip(trig)
                .map(|(&x, t)| {
                    let rect = t.abs().min(1.0);
                    let coeff = if rect > env { at } else { rt };
                    env = rect + coeff * (env - rect);
                    x * (1.0 - amount * env)
                })
                .collect()
        }
        Node::Lowpass { cutoff, q } => biquad(input, cutoff, *q, sr, FilterKind::Low),
        Node::Highpass { cutoff, q } => biquad(input, cutoff, *q, sr, FilterKind::High),
        Node::Bandpass { cutoff, q } => biquad(input, cutoff, *q, sr, FilterKind::Band),
        Node::Notch { cutoff, q } => biquad(input, cutoff, *q, sr, FilterKind::Notch),
        Node::Peak { cutoff, q, gain_db } => {
            biquad(input, cutoff, *q, sr, FilterKind::Peak(*gain_db))
        }
        Node::Lowshelf { cutoff, gain_db } => {
            biquad(input, cutoff, 0.707, sr, FilterKind::LowShelf(*gain_db))
        }
        Node::Highshelf { cutoff, gain_db } => {
            biquad(input, cutoff, 0.707, sr, FilterKind::HighShelf(*gain_db))
        }
        Node::Gain { amount } => {
            let g = eval_value(amount, input.len(), sr);
            input.iter().zip(g).map(|(x, k)| x * k).collect()
        }
        Node::Bitcrush { bits } => {
            let levels = (1u32 << *bits as u32) as f32;
            let half = levels / 2.0;
            input
                .iter()
                .map(|x| (x.clamp(-1.0, 1.0) * half).round() / half)
                .collect()
        }
        Node::Downsample { factor } => {
            let f = (*factor).max(1) as usize;
            let mut out = Vec::with_capacity(input.len());
            let mut held = 0.0;
            for (i, &x) in input.iter().enumerate() {
                if i % f == 0 {
                    held = x;
                }
                out.push(held);
            }
            out
        }
        Node::Delay { secs, feedback } => {
            // validate() caps secs at 30 s; the clamp guards direct render
            // calls on unvalidated docs from an unbounded allocation.
            let dn = ((secs.min(30.0) * sr as f32) as usize).max(1);
            let mut buf = vec![0.0f32; dn];
            let mut w = 0usize;
            let mut out = Vec::with_capacity(input.len());
            for &x in input {
                let delayed = buf[w];
                let y = x + feedback * delayed;
                buf[w] = y;
                w = (w + 1) % dn;
                out.push(y);
            }
            out
        }
        Node::Reverb { room, mix } => reverb(input, *room, *mix, sr, 0),
        Node::Modal { modes, mix } => modal_bank(input, modes, *mix, sr),
        Node::Drive { amount, shape, aa } => {
            let a = eval_value(amount, input.len(), sr);
            // ADAA is an engine-1 kernel: gated on the document's engine so
            // legacy (engine-0) documents render the original aliasing curve
            // byte-for-byte. Within engine 1 it is on unless `aa: false`.
            let use_adaa = engine >= 1 && aa.unwrap_or(true);
            if use_adaa {
                drive_adaa(input, &a, *shape)
            } else {
                input
                    .iter()
                    .zip(a)
                    .map(|(x, amt)| drive_curve(amt.max(0.0) * x, *shape))
                    .collect()
            }
        }
        Node::RingMod { freq } => {
            let f = eval_value(freq, input.len(), sr);
            let srf = sr as f32;
            let mut phase = 0.0f32;
            let mut out = Vec::with_capacity(input.len());
            for (i, &x) in input.iter().enumerate() {
                out.push(x * (TAU * phase).sin());
                phase += f[i].max(0.0) / srf;
                phase -= phase.floor();
            }
            out
        }
        Node::Chorus { rate, depth, mix } => chorus(input, *rate, *depth, *mix, sr),
        Node::Flanger {
            rate,
            depth,
            feedback,
            mix,
        } => flanger(input, *rate, *depth, *feedback, *mix, sr),
        Node::Phaser {
            rate,
            depth,
            feedback,
            mix,
        } => phaser(input, *rate, *depth, *feedback, *mix, sr),
        Node::Compress {
            threshold,
            ratio,
            attack,
            release,
            makeup,
        } => compress(input, *threshold, *ratio, *attack, *release, *makeup, sr),
        _ => input.to_vec(),
    }
}

/// ADSR envelope with an sfxr-style punch boost on the initial transient.
fn adsr(env: &Adsr, n: usize, sr: u32) -> Signal {
    let Adsr { a, d, s, r, punch } = *env;
    let srf = sr as f32;
    let rel_start = (n as f32 / srf - r).max(0.0);
    (0..n)
        .map(|i| crate::dsp::adsr_env(i as f32 / srf, a, d, s, r, punch, rel_start))
        .collect()
}
