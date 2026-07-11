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
//! Coverage is **every node type**:
//! - **Deterministic nodes** — oscillator sources (sine/square/triangle/sawtooth/
//!   fm/super), impact, env, all modulators, all filters + EQ, and all 12 effects
//!   (delay/reverb/modal/chorus/flanger/phaser/drive/ringmod/bitcrush/downsample/
//!   compress/duck), nested through mix/mul/chain — byte-identical *by construction*.
//! - **RNG nodes** (noise/dust/seq) under `engine >= 2`: each draws from its own
//!   structurally-seeded RNG (derived from its graph position), so the randomness
//!   is evaluation-order-independent and streams byte-identically. seq is
//!   pre-rendered with that seed via the exact offline synthesis and read back
//!   block-by-block.
//!
//! What falls back to the byte-identical buffer-backed
//! [`crate::player::Player`]: RNG nodes under `engine < 2` (they keep the old
//! shared, order-dependent stream); the **sampler** seq (an external stateful
//! rustysynth voice); a **`tracks` root** (the stereo mixer + master path — the
//! runtime's instance-per-layer model covers layering); a **`normalize`**
//! output stage (a whole-buffer op); **`loop` playback** (the crossfaded loop
//! body is a whole-buffer transform); and a **stereo** (Haas/Wide) treatment
//! (applied at write time, not in the graph).

mod proc;
mod source;
mod value;

#[cfg(test)]
mod tests;

use crate::dsl::{Node, Playback, SoundDoc, Stereo};
use crate::dsp::node_path;
use proc::{Proc, try_proc};
use source::{Src, try_src};

/// A stateful, block-by-block renderer for a supported graph.
pub struct StreamGraph {
    root: Src,
    pos: usize,
    /// Live note-pitch scale (1.0 = as authored). Smoothed per-sample toward
    /// `pitch_target` so a note change / portamento never zippers or clicks.
    pitch: f32,
    /// Where `pitch` is gliding to.
    pitch_target: f32,
    /// Per-sample one-pole glide coefficient in `(0, 1]`; `1.0` snaps instantly.
    glide: f32,
    /// Instant pitch-wheel multiplier, applied on top of `pitch`. Kept separate
    /// so bending never cancels an in-progress glide (and vice versa). The
    /// oscillators see `pitch * bend`.
    bend: f32,
}

impl StreamGraph {
    /// Build a streamer for `doc`, or `None` if the graph is outside the
    /// streamable subset — the caller then falls back to the buffer-backed
    /// [`crate::player::Player`].
    pub fn try_from_doc(doc: &SoundDoc) -> Option<Self> {
        // Loop docs render offline as their crossfaded loop body and stereo
        // (Haas/Wide) docs are stereoized at write time — neither transform
        // exists on the streaming path, so accepting them would play the raw
        // graph: un-looped, un-widened, and not byte-identical to the bounce.
        if doc.normalize.is_some()
            || matches!(doc.playback, Playback::Loop { .. })
            || !matches!(doc.stereo, Stereo::Mono)
            || matches!(doc.root, Node::Tracks { .. })
        {
            return None;
        }
        let n = ((doc.duration * doc.sample_rate as f32).ceil() as usize).max(1);
        Some(StreamGraph {
            root: try_src(
                &doc.root,
                doc.sample_rate,
                n,
                doc.effective_engine(),
                doc.seed,
            )?,
            pos: 0,
            pitch: 1.0,
            pitch_target: 1.0,
            glide: 1.0,
            bend: 1.0,
        })
    }

    /// Fill `out` with the next block of mono samples, advancing graph state.
    /// At the default pitch (1.0, no glide) this is bit-identical to the offline
    /// render — the pitch multiplier only bites once a caller bends or glides.
    pub fn fill(&mut self, out: &mut [f32]) {
        for s in out.iter_mut() {
            self.pitch += (self.pitch_target - self.pitch) * self.glide;
            *s = self.root.step(self.pos, self.pitch * self.bend);
            self.pos += 1;
        }
    }

    /// Set the pitch scale instantly (1.0 = as built, 2.0 = an octave up).
    /// Cancels any in-progress glide.
    pub fn set_pitch(&mut self, scale: f32) {
        self.pitch = scale.max(0.0);
        self.pitch_target = self.pitch;
        self.glide = 1.0;
    }

    /// Glide the pitch scale toward `scale` with a per-sample one-pole `coeff` in
    /// `(0, 1]` (`1.0` = instant). The target moves immediately; the audible pitch
    /// eases toward it, so a note change or pitch-wheel move never clicks.
    pub fn glide_pitch(&mut self, scale: f32, coeff: f32) {
        self.pitch_target = scale.max(0.0);
        self.glide = coeff.clamp(f32::MIN_POSITIVE, 1.0);
    }

    /// The note-pitch scale currently sounding (mid-glide, this trails the
    /// target). Excludes the bend multiplier.
    pub fn pitch(&self) -> f32 {
        self.pitch
    }

    /// Set the instant pitch-wheel multiplier (1.0 = centered), applied on top of
    /// the note pitch. Independent of glide, so a bend mid-portamento leaves the
    /// glide running.
    pub fn set_bend(&mut self, mul: f32) {
        self.bend = mul.max(0.0);
    }

    /// Sweep every filter's cutoff live — a brightness control. `scale`
    /// multiplies each biquad's cutoff (1.0 = as built); coefficients are
    /// recomputed in place, preserving state, so the sweep never clicks.
    /// Bit-identical to the built graph at `scale == 1.0`.
    pub fn set_cutoff(&mut self, scale: f32) {
        self.root.set_cutoff(scale.max(0.01));
    }
}

/// Whether `doc`'s graph can be streamed. A cheap check the runtime uses to pick
/// the streaming path.
pub fn is_streamable(doc: &SoundDoc) -> bool {
    StreamGraph::try_from_doc(doc).is_some()
}

/// A stateful chain of streaming effect processors applied to an input signal
/// block-by-block — byte-identical to the offline processors, carrying delay
/// lines / filter state across blocks. Used for a shared bus (e.g. an
/// instrument's master reverb/delay, so a tail is one shared instance rather than
/// one per voice).
pub struct EffectChain {
    procs: Vec<Proc>,
    pos: usize,
}

impl EffectChain {
    /// Build a chain from processor nodes at `sr`/`engine`, or `None` if any node
    /// isn't a streamable processor. (Modulated effect params are evaluated
    /// against a one-second reference for an `EnvMod` release anchor.)
    pub fn try_new(nodes: &[Node], sr: u32, engine: u32) -> Option<Self> {
        let n = sr as usize;
        let procs = nodes
            .iter()
            .enumerate()
            .map(|(i, node)| try_proc(node, sr, n, engine, node_path(0, i)))
            .collect::<Option<_>>()?;
        Some(EffectChain { procs, pos: 0 })
    }

    /// Process a mono block in place. The master bus isn't pitched, so processors
    /// run at the authored pitch (`1.0`).
    pub fn process(&mut self, block: &mut [f32]) {
        for x in block.iter_mut() {
            let mut v = *x;
            for p in self.procs.iter_mut() {
                v = p.step(v, self.pos, 1.0);
            }
            *x = v;
            self.pos += 1;
        }
    }

    /// Whether the chain has no processors.
    pub fn is_empty(&self) -> bool {
        self.procs.is_empty()
    }
}
