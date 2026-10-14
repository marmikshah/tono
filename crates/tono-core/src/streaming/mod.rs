//! The DSP evaluator: stateful, block-by-block rendering of a graph. The
//! offline bounce and realtime streaming run the same `Src`/`Proc` kernels —
//! the bounce is one block as long as the document, a stream is fixed-size
//! chunks — and every per-sample value is a function of the absolute sample
//! index plus state that advances in order, so the bytes never depend on how
//! the timeline was cut. Each stochastic leaf owns its RNG. Tracks carry named
//! RNG streams, automation, offsets, sidechains and stereo bus/master chains.
//!
//! Whole-buffer stages — normalization, loop extraction, Haas/Wide stereo
//! treatment, convolution, the granular texture, the SoundFont sampler — only
//! run in the bounce; [`StreamGraph::blockers`] names each one that keeps a
//! document from streaming, and the runtime then plays the buffer-backed
//! [`crate::player::Player`] instead.

mod proc;
mod source;
mod tracks;
pub(crate) mod value;

#[cfg(test)]
pub(crate) mod tests;

use std::fmt;

use crate::dsl::{Node, Playback, SeqWave, SoundDoc, Stereo};
use crate::dsp::node_path;
use proc::{Proc, try_proc};
use source::{Src, try_src};
pub(crate) use tracks::mix_offline;

/// Samples per internal block when streaming. Every scratch buffer is this
/// long, so `fill` never allocates whatever block size the host hands it.
pub(crate) const CHUNK: usize = 256;

/// What a graph is built for: the sample rate, the document length (the
/// closed-form envelopes key on it), the largest block `process` will see
/// (the scratch size), and whether whole-buffer stages may be built (the
/// bounce evaluates the whole document as one block).
#[derive(Clone, Copy)]
pub(crate) struct Ctx {
    pub(crate) sr: u32,
    pub(crate) n: usize,
    pub(crate) cap: usize,
    pub(crate) offline: bool,
}

/// An unvalidated document can carry any sample rate, and every delay line,
/// reverb and chorus buffer scales with it — a 30 s delay at `u32::MAX` Hz
/// would ask for 515 GB. The evaluator builds such a document at the
/// validation ceiling instead; validated documents never exceed it.
const MAX_BUILD_RATE: u32 = 192_000;

impl Ctx {
    /// The bounce: one block of `n` samples, every node allowed.
    pub(crate) fn offline(sr: u32, n: usize) -> Self {
        Ctx {
            sr: sr.min(MAX_BUILD_RATE),
            n,
            cap: n.max(1),
            offline: true,
        }
    }

    /// Streaming in [`CHUNK`]-sized blocks; whole-buffer nodes are refused.
    pub(crate) fn stream(sr: u32, n: usize) -> Self {
        Ctx {
            sr: sr.min(MAX_BUILD_RATE),
            n,
            cap: CHUNK,
            offline: false,
        }
    }

    pub(crate) fn srf(&self) -> f32 {
        self.sr as f32
    }
}

/// Evaluate `node` for `n` samples at `sr` in one block — the raw graph,
/// with no output stage. `path` is the structural seed of the root (the
/// document seed, or a track's stream seed).
pub(crate) fn render_graph_offline(node: &Node, sr: u32, n: usize, path: u64) -> Vec<f32> {
    let ctx = Ctx::offline(sr, n);
    let mut out = vec![0.0f32; n];
    // The bounce context builds every node; `None` would mean a variant the
    // walk does not know, which the exhaustive match in try_src rules out.
    if let Some(mut src) = try_src(node, &ctx, path) {
        let pitch = vec![1.0f32; n];
        src.process(&mut out, 0, &pitch);
    }
    out
}

/// Why a document can't stream — one entry per blocking feature, so an author
/// gets the reason and the fix instead of a silent fallback.
/// The `Display` text is the actionable message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StreamBlocker {
    /// A `normalize` output stage is a whole-buffer op.
    Normalize,
    /// `loop` playback renders as its crossfaded loop body — a whole-buffer
    /// transform.
    LoopPlayback,
    /// A Haas/Wide stereo treatment is applied at write time, not in the graph.
    StereoTreatment,
    /// A nested `tracks` mixer is unsupported.
    NestedMixer,
    /// A `tracks` mixer's part — a track's graph, a bus's insert
    /// chain, or the master chain — can't stream. Wraps the node-level cause
    /// with where it lives, so the author knows which channel to fix.
    TracksPart {
        /// Where the cause lives: `track '<layer id>'`, `bus '<bus id>'`, or
        /// `the master chain`.
        part: String,
        /// The node-level blocker the part trips.
        cause: Box<StreamBlocker>,
    },
    /// The SoundFont sampler seq is an external stateful voice.
    Sampler,
    /// An offline-only effect: convolution needs the whole input buffer at
    /// once, and the granular texture reads it out of order — neither can be
    /// evaluated per-sample.
    OfflineEffect {
        /// The node's `type` name (`convolve`, `granular`).
        name: &'static str,
    },
}

impl fmt::Display for StreamBlocker {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            StreamBlocker::Normalize => write!(
                f,
                "the normalize output stage is a whole-buffer op — bake the level into the graph (a gain node) instead"
            ),
            StreamBlocker::LoopPlayback => write!(
                f,
                "loop playback renders as its crossfaded loop body — stream the one-shot and loop at the host (Player), or bounce the loop offline"
            ),
            StreamBlocker::StereoTreatment => write!(
                f,
                "a Haas/Wide stereo treatment is applied at write time, not in the graph — stream mono and widen at the host"
            ),
            StreamBlocker::NestedMixer => write!(
                f,
                "nested tracks mixers are unsupported; use tracks only at the document root"
            ),
            StreamBlocker::TracksPart { part, cause } => write!(f, "{part}: {cause}"),
            StreamBlocker::Sampler => write!(
                f,
                "the SoundFont sampler seq is an external stateful voice — bounce the part offline, or voice it with the built-in waves"
            ),
            StreamBlocker::OfflineEffect { name } => write!(
                f,
                "{name} is an offline effect — it needs the whole input buffer; bounce it offline / apply it at bounce time and keep the streamed graph causal"
            ),
        }
    }
}

/// The node-level streaming blockers of one graph, pushed through `push` —
/// the per-node rules shared by the plain-document walk and the mixer's
/// per-part walk (which wraps each cause with its track/bus context).
fn node_blockers(node: &Node, push: &mut impl FnMut(StreamBlocker)) {
    match node {
        // A nested mixer (validation rejects it) can't stream either.
        Node::Tracks { .. } => push(StreamBlocker::NestedMixer),
        Node::Seq {
            wave: SeqWave::Sampler,
            ..
        } => push(StreamBlocker::Sampler),
        // Offline-only whole-buffer effects (FFT convolution; out-of-order
        // granular reads) — no per-sample streaming form exists.
        Node::Convolve { .. } => {
            push(StreamBlocker::OfflineEffect { name: "convolve" });
        }
        Node::Granular { .. } => {
            push(StreamBlocker::OfflineEffect { name: "granular" });
        }
        _ => {}
    }
}

/// A stateful, block-by-block renderer for a supported graph.
pub struct StreamGraph {
    root: Root,
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
    /// The per-sample `pitch * bend` of the block in flight.
    pitch_buf: Vec<f32>,
    /// A mixer's stereo block, for the mono (mid) fill.
    scratch_l: Vec<f32>,
    scratch_r: Vec<f32>,
}

/// The streamed root: a plain mono graph, or a `tracks` mixer.
enum Root {
    Mono(Src),
    Tracks(tracks::StreamTracks),
}

impl StreamGraph {
    /// Why `doc` can't stream — one entry per blocking feature (doc-level
    /// first, then nodes in walk order), empty when [`try_from_doc`](Self::try_from_doc)
    /// would succeed. The actionable companion to the silent `Option`: the
    /// Engine/StreamSource fallback path stays allocation-free, and authors
    /// get the reason and the fix. A `tracks` root streams natively
    /// when every part does, so its blockers name the failing part (track,
    /// bus, or master chain) with the node-level cause.
    pub fn blockers(doc: &SoundDoc) -> Vec<StreamBlocker> {
        let mut out = Vec::new();
        if doc.normalize.is_some() {
            out.push(StreamBlocker::Normalize);
        }
        if matches!(doc.playback, Playback::Loop { .. }) {
            out.push(StreamBlocker::LoopPlayback);
        }
        if !matches!(doc.stereo, Stereo::Mono) {
            out.push(StreamBlocker::StereoTreatment);
        }

        match &doc.root {
            Node::Tracks {
                tracks,
                master,
                buses,
            } => {
                // The mixer streams natively when every part does —
                // report the failing part with its context instead of the
                // blanket NestedMixer.
                for (ti, t) in tracks.iter().enumerate() {
                    let part = format!("track '{}'", t.layer_id(ti));
                    t.node.walk(&mut |node| {
                        node_blockers(node, &mut |cause| {
                            out.push(StreamBlocker::TracksPart {
                                part: part.clone(),
                                cause: Box::new(cause),
                            });
                        });
                    });
                }
                for m in master {
                    m.walk(&mut |node| {
                        node_blockers(node, &mut |cause| {
                            out.push(StreamBlocker::TracksPart {
                                part: "the master chain".to_string(),
                                cause: Box::new(cause),
                            });
                        });
                    });
                }
                for b in buses {
                    let part = format!("bus '{}'", b.id);
                    for fx in &b.effects {
                        fx.walk(&mut |node| {
                            node_blockers(node, &mut |cause| {
                                out.push(StreamBlocker::TracksPart {
                                    part: part.clone(),
                                    cause: Box::new(cause),
                                });
                            });
                        });
                    }
                }
            }
            _ => doc
                .root
                .walk(&mut |node| node_blockers(node, &mut |b| out.push(b))),
        }
        out.dedup();
        out
    }

    /// Build a streamer for `doc`, or `None` if the graph is outside the
    /// streamable subset — the caller then falls back to the buffer-backed
    /// [`crate::player::Player`]. [`blockers`](Self::blockers) says why.
    pub fn try_from_doc(doc: &SoundDoc) -> Option<Self> {
        if !Self::blockers(doc).is_empty() {
            return None;
        }
        let ctx = Ctx::stream(doc.sample_rate, doc.frame_count());

        let root = if matches!(doc.root, Node::Tracks { .. }) {
            Root::Tracks(tracks::StreamTracks::build(doc, &ctx, false, false)?)
        } else {
            Root::Mono(try_src(&doc.root, &ctx, doc.seed)?)
        };
        Some(StreamGraph {
            root,
            pos: 0,

            pitch: 1.0,
            pitch_target: 1.0,
            glide: 1.0,
            bend: 1.0,
            pitch_buf: vec![1.0; CHUNK],
            scratch_l: vec![0.0; CHUNK],
            scratch_r: vec![0.0; CHUNK],
        })
    }

    /// Advance the glide for the next `len` samples, writing the effective
    /// per-sample pitch into `pitch_buf[..len]`.
    fn glide_block(&mut self, len: usize) {
        for p in self.pitch_buf[..len].iter_mut() {
            self.pitch += (self.pitch_target - self.pitch) * self.glide;
            *p = self.pitch * self.bend;
        }
    }

    /// Fill `out` with the next block of mono samples, advancing graph state.
    /// At the default pitch (1.0, no glide) this is bit-identical to the offline
    /// render — the pitch multiplier only bites once a caller bends or glides.
    /// A `tracks` document fills its mid (`0.5 × (L + R)`, what
    /// [`crate::render::render_product`] hands mono consumers).
    pub fn fill(&mut self, out: &mut [f32]) {
        for chunk in out.chunks_mut(CHUNK) {
            let len = chunk.len();
            self.glide_block(len);
            let pitch = &self.pitch_buf[..len];
            match &mut self.root {
                Root::Mono(src) => src.process(chunk, self.pos, pitch),
                Root::Tracks(mix) => {
                    let (l, r) = (&mut self.scratch_l[..len], &mut self.scratch_r[..len]);
                    mix.process(l, r, self.pos, pitch);
                    for ((o, a), b) in chunk.iter_mut().zip(l.iter()).zip(r.iter()) {
                        *o = 0.5 * (a + b);
                    }
                }
            }
            self.pos += len;
        }
    }

    /// Fill `left` / `right` with the next block of the stereo bus. A plain
    /// document is mono — both channels carry the same signal (exactly what
    /// [`crate::runtime::StreamSource`] duplicates today); a `tracks`
    /// document produces its real stereo image, bit-identical to the offline
    /// mixer's bus. The slice lengths must match.
    pub fn fill_stereo(&mut self, left: &mut [f32], right: &mut [f32]) {
        assert_eq!(left.len(), right.len(), "stereo blocks must match");
        for (l, r) in left.chunks_mut(CHUNK).zip(right.chunks_mut(CHUNK)) {
            let len = l.len();
            self.glide_block(len);
            let pitch = &self.pitch_buf[..len];
            match &mut self.root {
                Root::Mono(src) => {
                    src.process(l, self.pos, pitch);
                    r.copy_from_slice(l);
                }
                Root::Tracks(mix) => mix.process(l, r, self.pos, pitch),
            }
            self.pos += len;
        }
    }

    /// True when the streamed document renders a real stereo image (a
    /// `tracks` mixer) — `fill_stereo` then produces distinct
    /// channels, and the bounce's peak limit is measured over both.
    pub fn is_stereo(&self) -> bool {
        matches!(self.root, Root::Tracks(_))
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
        // clamp() passes NaN through, and a NaN glide would latch pitch to NaN
        // forever — fold it to an instant snap instead.
        self.glide = if coeff.is_nan() {
            1.0
        } else {
            coeff.clamp(f32::MIN_POSITIVE, 1.0)
        };
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
        let scale = scale.max(0.01);
        match &mut self.root {
            Root::Mono(src) => src.set_cutoff(scale),
            Root::Tracks(mix) => mix.set_cutoff(scale),
        }
    }
}

/// A stateful chain of effect processors applied to a signal block by block,
/// carrying delay lines / filter state across blocks. Used for a shared bus
/// (e.g. an instrument's master reverb/delay, so a tail is one shared
/// instance rather than one per voice).
pub struct EffectChain {
    procs: Vec<Proc>,
    pos: usize,
    /// A block of 1.0 pitch multipliers: a bus is never pitched.
    ones: Vec<f32>,
}

impl EffectChain {
    /// Build an effect chain at `sr`; returns None for a non-streamable node.
    pub fn try_new(nodes: &[Node], sr: u32) -> Option<Self> {
        let ctx = Ctx::stream(sr, sr as usize);
        let procs = nodes
            .iter()
            .enumerate()
            .map(|(i, node)| try_proc(node, &ctx, node_path(0, i)))
            .collect::<Option<_>>()?;
        Some(EffectChain {
            procs,
            pos: 0,
            ones: vec![1.0; CHUNK],
        })
    }

    /// Process a mono block in place.
    pub fn process(&mut self, block: &mut [f32]) {
        for chunk in block.chunks_mut(CHUNK) {
            let len = chunk.len();
            for p in self.procs.iter_mut() {
                p.process(chunk, self.pos, &self.ones[..len]);
            }
            self.pos += len;
        }
    }
}
