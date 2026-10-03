//! The streaming mixer: a `tracks` root rendered block-by-block,
//! byte-identical to the offline [`crate::render::render_tracks`].
//!
//! The offline mixer is a two-pass whole-buffer render (pass 1 renders every
//! track's graph, pass 2 mixes). Every per-sample value it computes is a
//! deterministic function of the absolute bus position plus state that
//! evolves sample-by-sample in order (the track graphs, the duck envelopes,
//! the bus/master inserts) — so the mix runs here as ONE per-sample loop,
//! and chunking it into blocks of any size can't change the bytes (the same
//! argument the rest of the streaming renderer rests on). Per bus position
//! `p`, in the offline's exact order:
//!
//! 1. Each track's graph is stepped for its local sample `p - at`. Each
//!    track has an id-keyed RNG stream, so stepping the same
//!    graph with the same seed reproduces the offline pass-1 render exactly.
//!    A muted track contributes exact zeros and its graph is never built —
//!    the offline renderer also skips its graph.
//! 2. Each sidechain follower's duck envelope advances one step, driven by
//!    its source's positioned post-fader signal (the source's raw sample ×
//!    its gain fader at `p`, pre-pan — the offline's `duck_envelope`).
//! 3. Each track's post-fader contribution pans onto its destination (the
//!    master bus, or its named bus) plus a scaled copy per send — in track
//!    declaration order, the offline's per-sample accumulation order.
//! 4. Each bus's insert chain processes one sample per channel (a reverb
//!    gets the 0/23 decorrelated-tails spread, like the offline), then the
//!    return fader lands on the master bus — in bus declaration order.
//! 5. The master chain processes one sample per channel.
//!
//! The final `peak_limit` stays outside the graph (a whole-buffer gain): the
//! runtime's `StreamSource` probe measures and bakes it, exactly as it does
//! for plain documents.

use super::proc::{Proc, reverb_proc, try_proc};
use super::source::{Src, try_src};
use crate::dsl::{AutoTarget, Node, SoundDoc};
use crate::dsp::layer_stream_key;
use crate::render::{LaneCursor, MASTER_STREAM, pan_gains, track_stream_seed};

/// One mixer channel's streaming state.
struct TrackStream {
    /// The track's graph. `None` when muted — a muted track draws nothing
    /// and contributes exact silence (the offline's `TrackRender::Muted`).
    src: Option<Src>,
    /// The `at` start offset, in samples.
    off: usize,
    /// The static equal-power gains, precomputed once — the offline's
    /// constant fast path when no lane drives the fader or the pan.
    constant: (f32, f32),
    /// The static fader / pan, for the unautomated arm of the lane lookup.
    gain: f32,
    pan: f32,
    /// Automation cursors over the bus (song) timeline; `None` ⇒ the static
    /// value applies. Values land at the same positions the offline's
    /// whole-buffer `lane_for` scan produces.
    gain_lane: Option<LaneCursor>,
    pan_lane: Option<LaneCursor>,
    /// Main-output destination: `None` = the master bus, `Some(bi)` = bus
    /// `bi`. A dangling `bus` id resolves to the master, as the offline does.
    dest: Option<usize>,
    /// Resolved post-fader sends `(bus index, amount)`; dangling sends are
    /// dropped at build, exactly as the offline ignores them.
    sends: Vec<(usize, f32)>,
}

/// A resolved sidechain link: one follower's duck envelope, driven by the
/// source track's positioned post-fader signal. The follower state carries
/// across blocks (the `duck` node's exact attack/release recurrence).
struct DuckLink {
    /// The ducked track's index.
    follower: usize,
    /// The driving track's index.
    source: usize,
    /// The source's static fader, and its gain-lane cursor — the offline
    /// evaluates the source's gain lane inside `duck_envelope`.
    source_gain: f32,
    gain_lane: Option<LaneCursor>,
    /// Follower state and coefficients.
    env: f32,
    at: f32,
    rt: f32,
    amount: f32,
}

/// A stereo insert chain (a bus's inserts or the master chain): every
/// processor runs as a per-channel pair, and a reverb gets the 0/23
/// decorrelated-tails spread — the offline bus/master treatment. RNG leaves
/// self-seed structurally, so two identically-built instances reproduce
/// the offline left/right passes bit-for-bit.
struct StereoChain {
    fx: Vec<(Proc, Proc)>,
    /// Absolute position on the bus timeline (the closed-form processors —
    /// tremolo, chorus, … — key on it).
    pos: usize,
}

impl StereoChain {
    /// Build the per-channel pairs, or `None` if any node isn't a streamable
    /// processor. `path` is the bus's (or master's) stream seed — the same
    /// path the offline hands every insert in the chain, so e.g. a `duck`'s
    /// trigger seeds identically.
    fn build(nodes: &[Node], sr: u32, n: usize, path: u64) -> Option<Self> {
        let fx = nodes
            .iter()
            .map(|node| {
                if let Node::Reverb { room, mix } = node {
                    Some((
                        reverb_proc(*room, *mix, sr, 0),
                        reverb_proc(*room, *mix, sr, 23),
                    ))
                } else {
                    Some((try_proc(node, sr, n, path)?, try_proc(node, sr, n, path)?))
                }
            })
            .collect::<Option<_>>()?;
        Some(StereoChain { fx, pos: 0 })
    }

    /// Process one stereo sample.
    fn step(&mut self, l: f32, r: f32) -> (f32, f32) {
        let pos = self.pos;
        self.pos += 1;

        let (mut l, mut r) = (l, r);
        for (pl, pr) in self.fx.iter_mut() {
            l = pl.step(l, pos, 1.0);
            r = pr.step(r, pos, 1.0);
        }
        (l, r)
    }

    fn set_cutoff(&mut self, scale: f32) {
        for (pl, pr) in self.fx.iter_mut() {
            pl.set_cutoff(scale);
            pr.set_cutoff(scale);
        }
    }
}

/// One mix bus: its insert chain and the return fader.
struct BusStream {
    chain: StereoChain,
    gain: f32,
}

/// The streaming `tracks` mixer — see the module docs for the pipeline.
pub(crate) struct StreamTracks {
    tracks: Vec<TrackStream>,
    links: Vec<DuckLink>,
    buses: Vec<BusStream>,
    master: StereoChain,
    /// Per-track current raw sample / current duck value (reused per sample).
    xs: Vec<f32>,
    ducks: Vec<f32>,
    /// Per-bus current input sample (this position's routed + sent sum).
    bus_in: Vec<(f32, f32)>,
    sr: u32,
    /// The bus position (absolute sample index on the song timeline).
    pos: usize,
}

impl StreamTracks {
    /// Build the mixer for a `tracks` document, or `None` if any
    /// part isn't streamable ([`StreamGraph::blockers`](super::StreamGraph::blockers)
    /// reports the same failures with their track/bus context, so the two
    /// stay in agreement).
    pub(crate) fn build(doc: &SoundDoc) -> Option<Self> {
        let Node::Tracks {
            tracks,
            master,
            buses,
        } = &doc.root
        else {
            return None;
        };

        let sr = doc.sample_rate;
        // The duration clamp mirrors the offline render paths so an
        // unvalidated doc can't request an unbounded seq pre-render here.
        let n = ((doc.duration.clamp(0.0, 600.0) * sr as f32).ceil() as usize).max(1);

        let bus_index = |id: &str| buses.iter().position(|b| b.id == id);
        let mut out_tracks = Vec::with_capacity(tracks.len());
        let mut links = Vec::new();
        for (ti, t) in tracks.iter().enumerate() {
            // The id fallback mirrors the offline (the exact id
            // `ensure_track_ids` backfills), so a document's noise is
            // identical before and after the backfill pass.
            let layer_id = t.id.clone().unwrap_or_else(|| format!("layer_{ti}"));
            let base = track_stream_seed(doc.seed, layer_stream_key(&layer_id));
            let off = ((t.at.max(0.0) * sr as f32).round() as usize).min(n);
            let src = if t.mute {
                None
            } else {
                Some(try_src(&t.node, sr, n, base)?)
            };
            // The sidechain resolves against the DECLARED id only — a
            // backfilled `layer_<i>` id is not a match (the offline's exact
            // rule). A missing source means no ducking (validate rejects the
            // document; unvalidated renders duck nothing).
            if let Some(sc) = &t.sidechain
                && let Some((si, source)) = tracks
                    .iter()
                    .enumerate()
                    .find(|(_, s)| s.id.as_deref() == Some(sc.source.as_str()))
            {
                let srf = sr as f32;
                links.push(DuckLink {
                    follower: ti,
                    source: si,
                    source_gain: source.gain,
                    gain_lane: LaneCursor::build(&source.automation, AutoTarget::Gain, source.gain),
                    env: 0.0,
                    at: crate::dsp::exp(-1.0 / (sc.attack.max(1e-4) * srf)),
                    rt: crate::dsp::exp(-1.0 / (sc.release.max(1e-4) * srf)),
                    amount: sc.amount,
                });
            }
            out_tracks.push(TrackStream {
                src,
                off,
                constant: pan_gains(t.pan.clamp(-1.0, 1.0), t.gain),
                gain: t.gain,
                pan: t.pan,
                gain_lane: LaneCursor::build(&t.automation, AutoTarget::Gain, t.gain),
                pan_lane: LaneCursor::build(&t.automation, AutoTarget::Pan, t.pan),
                dest: t.bus.as_deref().and_then(bus_index),
                sends: t
                    .sends
                    .iter()
                    .filter_map(|s| bus_index(&s.bus).map(|bi| (bi, s.amount.clamp(0.0, 1.0))))
                    .collect(),
            });
        }
        let buses: Vec<BusStream> = buses
            .iter()
            .map(|b| {
                let bpath = track_stream_seed(doc.seed, layer_stream_key(&format!("bus:{}", b.id)));
                Some(BusStream {
                    chain: StereoChain::build(&b.effects, sr, n, bpath)?,
                    gain: if b.gain.is_finite() { b.gain } else { 1.0 },
                })
            })
            .collect::<Option<_>>()?;
        let master = StereoChain::build(master, sr, n, track_stream_seed(doc.seed, MASTER_STREAM))?;
        let n_tracks = out_tracks.len();
        let n_buses = buses.len();
        Some(StreamTracks {
            tracks: out_tracks,
            links,
            buses,
            master,
            xs: vec![0.0; n_tracks],
            ducks: vec![1.0; n_tracks],
            bus_in: vec![(0.0, 0.0); n_buses],
            sr,

            pos: 0,
        })
    }

    /// One bus sample at the current position: the whole mix pipeline
    /// (sources → ducks → faders → buses → master chain), advancing every
    /// carried state. `pitch` is the live pitch scale (1.0 = as authored;
    /// the bounce is pitch 1.0, where this is bit-identical to the offline
    /// mixer).
    pub(crate) fn step(&mut self, pitch: f32) -> (f32, f32) {
        let p = self.pos;
        self.pos += 1;
        let sr = self.sr;

        // 1. Track graphs. A track sounds at local sample p - off; before its
        //    `at` lands it contributes nothing and its graph is not stepped
        //    (the first stepped sample is its local 0, exactly the offline's
        //    render-then-shift).
        for (i, t) in self.tracks.iter_mut().enumerate() {
            self.xs[i] = match &mut t.src {
                Some(src) if p >= t.off => src.step(p - t.off, pitch),
                _ => 0.0,
            };
        }
        // 2. Duck envelopes, driven by the source's positioned post-fader
        //    signal (raw × gain fader at this bus position, pre-pan). A muted
        //    source feeds exact zeros — the envelope stays open — and its
        //    lane is never scanned, matching the offline's Muted arm.
        for d in self.ducks.iter_mut() {
            *d = 1.0;
        }
        for link in self.links.iter_mut() {
            let sig = if self.tracks[link.source].src.is_some() && p >= self.tracks[link.source].off
            {
                let g = link
                    .gain_lane
                    .as_mut()
                    .map_or(link.source_gain, |c| c.at(p, sr));
                self.xs[link.source] * g
            } else {
                0.0
            };
            let rect = sig.abs().min(1.0);
            let coeff = if rect > link.env { link.at } else { link.rt };
            link.env = rect + coeff * (link.env - rect);
            self.ducks[link.follower] = 1.0 - link.amount * link.env;
        }
        // 3. The mix pass, in track declaration order: pan/gain (the offline's
        //    constant fast path when no lane drives them), the duck, then the
        //    destination and the post-fader sends.
        let (mut ml, mut mr) = (0.0f32, 0.0f32);
        for b in self.bus_in.iter_mut() {
            *b = (0.0, 0.0);
        }
        for (i, t) in self.tracks.iter_mut().enumerate() {
            if t.src.is_none() || p < t.off {
                continue;
            }
            let x = self.xs[i];
            let (gl, gr) = match (&mut t.gain_lane, &mut t.pan_lane) {
                (None, None) => t.constant,
                (g, pn) => {
                    let gain = g.as_mut().map_or(t.gain, |c| c.at(p, sr));
                    let pan = pn.as_mut().map_or(t.pan, |c| c.at(p, sr)).clamp(-1.0, 1.0);
                    pan_gains(pan, gain)
                }
            };
            let d = self.ducks[i];
            let (la, ra) = (x * gl * d, x * gr * d);
            match t.dest {
                None => {
                    ml += la;
                    mr += ra;
                }
                Some(bi) => {
                    self.bus_in[bi].0 += la;
                    self.bus_in[bi].1 += ra;
                }
            }
            for (bi, amount) in &t.sends {
                self.bus_in[*bi].0 += la * amount;
                self.bus_in[*bi].1 += ra * amount;
            }
        }
        // 4. Buses, in declaration order: the insert chain, then the return
        //    fader onto the master bus (full range, like the offline).
        for (bi, b) in self.buses.iter_mut().enumerate() {
            let (bl, br) = b.chain.step(self.bus_in[bi].0, self.bus_in[bi].1);
            ml += bl * b.gain;
            mr += br * b.gain;
        }
        // 5. The master chain.
        self.master.step(ml, mr)
    }

    /// Sweep every filter's cutoff live, track graphs and insert chains
    /// alike (see [`StreamGraph::set_cutoff`](super::StreamGraph::set_cutoff)).
    pub(crate) fn set_cutoff(&mut self, scale: f32) {
        for t in self.tracks.iter_mut() {
            if let Some(src) = &mut t.src {
                src.set_cutoff(scale);
            }
        }
        for b in self.buses.iter_mut() {
            b.chain.set_cutoff(scale);
        }
        self.master.set_cutoff(scale);
    }
}
