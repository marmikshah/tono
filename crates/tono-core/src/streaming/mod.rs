//! Stateful, block-by-block rendering of causal graphs, byte-identical to the
//! offline renderer at every block size. Each stochastic leaf owns its RNG.
//! Tracks carry named RNG streams, automation, offsets, sidechains and stereo
//! bus/master chains across blocks. Seq voices are pre-rendered at construction.
//!
//! Whole-buffer normalization, loop extraction, Haas/Wide stereo treatment,
//! SoundFont sampling, convolution and granular effects use the buffer-backed
//! Player. `StreamGraph::blockers` describes the cause of each fallback.

mod proc;
mod source;
mod tracks;
pub(crate) mod value;

#[cfg(test)]
pub(crate) mod tests;

use std::fmt;

use crate::dsl::{Node, Playback, SeqWave, SoundDoc, Stereo, Value};
use crate::dsp::node_path;
use proc::{Proc, try_proc};
use source::{Src, try_src};

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
    /// A filter / EQ / gain carries a modulated cutoff or amount — the
    /// streaming biquads hold constant coefficients.
    ModulatedFilter,
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
            StreamBlocker::ModulatedFilter => write!(
                f,
                "a filter/EQ/gain carries a modulated cutoff or amount — bake it constant, or sweep it live with set_cutoff"
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
        // Filters/EQ/gain only stream with a constant cutoff/amount (the
        // streaming biquads hold constant coefficients).
        Node::Gain {
            amount: Value::Modulated(_),
        } => {
            push(StreamBlocker::ModulatedFilter);
        }
        Node::Lowpass {
            cutoff: Value::Modulated(_),
            ..
        }
        | Node::Highpass {
            cutoff: Value::Modulated(_),
            ..
        }
        | Node::Bandpass {
            cutoff: Value::Modulated(_),
            ..
        }
        | Node::Notch {
            cutoff: Value::Modulated(_),
            ..
        }
        | Node::Peak {
            cutoff: Value::Modulated(_),
            ..
        }
        | Node::Lowshelf {
            cutoff: Value::Modulated(_),
            ..
        }
        | Node::Highshelf {
            cutoff: Value::Modulated(_),
            ..
        } => {
            push(StreamBlocker::ModulatedFilter);
        }
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
                    // The id fallback mirrors the renderer's (and
                    // `ensure_track_ids`) — the same id the fix addresses.
                    let layer_id = t.id.clone().unwrap_or_else(|| format!("layer_{ti}"));
                    let part = format!("track '{layer_id}'");
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
        // The duration clamp mirrors the offline render paths so an
        // unvalidated doc can't request an unbounded seq pre-render here.
        let n = ((doc.duration.clamp(0.0, 600.0) * doc.sample_rate as f32).ceil() as usize).max(1);

        let root = if matches!(doc.root, Node::Tracks { .. }) {
            Root::Tracks(tracks::StreamTracks::build(doc)?)
        } else {
            Root::Mono(try_src(&doc.root, doc.sample_rate, n, doc.seed)?)
        };
        Some(StreamGraph {
            root,
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
    /// A `tracks` document fills its mid (`0.5 × (L + R)`, what
    /// [`crate::render::render_product`] hands mono consumers).
    pub fn fill(&mut self, out: &mut [f32]) {
        for s in out.iter_mut() {
            self.pitch += (self.pitch_target - self.pitch) * self.glide;
            let pitch = self.pitch * self.bend;
            *s = match &mut self.root {
                Root::Mono(src) => src.step(self.pos, pitch),
                Root::Tracks(mix) => {
                    let (l, r) = mix.step(pitch);
                    0.5 * (l + r)
                }
            };
            self.pos += 1;
        }
    }

    /// Fill `left` / `right` with the next block of the stereo bus. A plain
    /// document is mono — both channels carry the same signal (exactly what
    /// [`crate::runtime::StreamSource`] duplicates today); a `tracks`
    /// document produces its real stereo image, bit-identical to the offline
    /// mixer's bus. The slice lengths must match.
    pub fn fill_stereo(&mut self, left: &mut [f32], right: &mut [f32]) {
        assert_eq!(left.len(), right.len(), "stereo blocks must match");

        for (l, r) in left.iter_mut().zip(right.iter_mut()) {
            self.pitch += (self.pitch_target - self.pitch) * self.glide;
            let pitch = self.pitch * self.bend;
            match &mut self.root {
                Root::Mono(src) => {
                    let v = src.step(self.pos, pitch);
                    *l = v;
                    *r = v;
                }
                Root::Tracks(mix) => {
                    let (a, b) = mix.step(pitch);
                    *l = a;
                    *r = b;
                }
            }
            self.pos += 1;
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
    /// Build an effect chain at `sr`; returns None for a non-streamable node.
    pub fn try_new(nodes: &[Node], sr: u32) -> Option<Self> {
        let n = sr as usize;
        let procs = nodes
            .iter()
            .enumerate()
            .map(|(i, node)| try_proc(node, sr, n, node_path(0, i)))
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
}
