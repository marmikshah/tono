//! The `tracks` mixer, block by block. Per block, in stages that keep every
//! per-sample value a function of the absolute bus position plus state that
//! advances in order, so the bounce (one block) and a stream (any block
//! size) produce the same bytes:
//!
//! 1. Each track's graph is evaluated for its local samples `p - at` (a
//!    track contributes nothing before its `at`, and its graph is not
//!    stepped). Each track has an id-keyed RNG stream. A muted track draws
//!    nothing and its graph is never built.
//! 2. Each sidechain follower's duck envelope advances, driven by its
//!    source's positioned post-fader signal (raw × gain fader, pre-pan).
//! 3. Each track's post-fader contribution pans onto its destination (the
//!    master bus or its named bus) plus a scaled copy per send, in track
//!    declaration order. The bounce captures per-layer stats and stems here.
//! 4. Each bus's insert chain processes its block per channel (a reverb gets
//!    the 0/23 decorrelated-tails spread), then the return fader lands on the
//!    master bus, in bus declaration order.
//! 5. The master chain processes the block.
//!
//! The final `peak_limit` stays outside the graph (a whole-buffer gain): the
//! bounce applies it in the output stage and the runtime's `StreamSource`
//! probe measures and bakes it.

use super::Ctx;
use super::EffectChain;
use super::source::{Src, try_src};
use crate::dsl::{AutoTarget, Node, SoundDoc};
use crate::dsp::layer_stream_key;
use crate::render::{
    LaneCursor, LayerStats, Stem, pan_gains, track_native_stereo, track_stream_seed,
};

/// What a mixer channel plays.
enum Voice {
    /// A muted track contributes exact silence.
    Muted,
    Mono(Src),
    /// A native-stereo (SoundFont sampler) track, pre-rendered for the bounce.
    Stereo(Vec<f32>, Vec<f32>),
}

/// One mixer channel's state.
struct TrackStream {
    voice: Voice,
    /// The `at` start offset, in samples.
    off: usize,
    /// The static equal-power gains, precomputed once — the constant fast
    /// path when no lane drives the fader or the pan.
    constant: (f32, f32),
    gain: f32,
    pan: f32,
    /// Automation cursors over the bus timeline; `None` ⇒ the static value.
    gain_lane: Option<LaneCursor>,
    pan_lane: Option<LaneCursor>,
    /// Main-output destination: `None` = the master bus, `Some(bi)` = bus
    /// `bi`. A dangling `bus` id resolves to the master.
    dest: Option<usize>,
    /// The authored bus id, reported on the track's stem.
    bus_id: Option<String>,
    /// Resolved post-fader sends `(bus index, amount)`; dangling sends are
    /// dropped at build.
    sends: Vec<(usize, f32)>,
}

/// A resolved sidechain link: one follower's duck envelope, driven by the
/// source track's positioned post-fader signal through an attack/release
/// follower.
struct DuckLink {
    follower: usize,
    source: usize,
    source_gain: f32,
    gain_lane: Option<LaneCursor>,
    env: f32,
    at: f32,
    rt: f32,
    amount: f32,
}

/// A stereo insert chain (a bus's inserts or the master chain): every
/// processor runs as a per-channel pair, and a reverb gets the 0/23
/// decorrelated-tails spread.
struct StereoChain(EffectChain, EffectChain);

impl StereoChain {
    fn build(nodes: &[Node], ctx: &Ctx) -> Option<Self> {
        let (l, r) = EffectChain::stereo(nodes, ctx.sr)?;
        Some(StereoChain(l, r))
    }

    fn process(&mut self, l: &mut [f32], r: &mut [f32]) {
        self.0.process(l);
        self.1.process(r);
    }

    fn set_cutoff(&mut self, scale: f32) {
        self.0.set_cutoff(scale);
        self.1.set_cutoff(scale);
    }
}

/// One mix bus: its insert chain and the return fader.
struct BusStream {
    id: String,
    chain: StereoChain,
    gain: f32,
}

/// Per-layer contribution accumulators (post fader/pan/offset/duck, pre
/// bus/master), measured over the whole timeline.
struct LayerAcc {
    id: String,
    mute: bool,
    peak: f32,
    energy: f64,
}

/// The bounce's stem capture: every track's positioned contribution and
/// every bus's processed return, over the whole document.
struct StemBufs {
    tracks: Vec<(Vec<f32>, Vec<f32>)>,
    buses: Vec<(Vec<f32>, Vec<f32>)>,
}

/// The bounce-only measurements taken in the mix pass.
struct Capture {
    layers: Vec<LayerAcc>,
    stems: Option<StemBufs>,
}

/// The `tracks` mixer — see the module docs for the pipeline.
pub(crate) struct StreamTracks {
    tracks: Vec<TrackStream>,
    links: Vec<DuckLink>,
    buses: Vec<BusStream>,
    master: StereoChain,
    /// Per-track raw block and duck multipliers for the block in flight.
    xs: Vec<Vec<f32>>,
    ducks: Vec<Vec<f32>>,
    /// Per-bus input block (this block's routed + sent sum), left and right.
    bus_l: Vec<Vec<f32>>,
    bus_r: Vec<Vec<f32>>,
    sr: u32,
    capture: Option<Capture>,
}

impl StreamTracks {
    /// Build the mixer for a `tracks` document, or `None` if the context
    /// refuses a part ([`StreamGraph::blockers`](super::StreamGraph::blockers)
    /// reports the same failures with their track/bus context). `capture`
    /// records layer stats (and stems when `stems`) for the bounce.
    pub(crate) fn build(doc: &SoundDoc, ctx: &Ctx, capture: bool, stems: bool) -> Option<Self> {
        let Node::Tracks {
            tracks,
            master,
            buses,
        } = &doc.root
        else {
            return None;
        };
        let sr = ctx.sr;
        let n = ctx.n;

        let bus_index = |id: &str| buses.iter().position(|b| b.id == id);
        let mut out_tracks = Vec::with_capacity(tracks.len());
        let mut links = Vec::new();
        let mut layers = Vec::with_capacity(tracks.len());
        for (ti, t) in tracks.iter().enumerate() {
            let layer_id = t.layer_id(ti);
            let base = track_stream_seed(doc.seed, layer_stream_key(&layer_id));
            let off = ((t.at.max(0.0) * sr as f32).round() as usize).min(n);
            let voice = if t.mute {
                Voice::Muted
            } else if let Some((l, r)) = ctx
                .offline
                .then(|| track_native_stereo(&t.node, n, sr))
                .flatten()
            {
                Voice::Stereo(l, r)
            } else {
                Voice::Mono(try_src(&t.node, ctx, base)?)
            };
            // The sidechain resolves against the DECLARED id only — a
            // backfilled `layer_<i>` id is not a match. A missing source means
            // no ducking (validate rejects the document).
            if let Some(sc) = &t.sidechain
                && let Some((si, source)) = tracks
                    .iter()
                    .enumerate()
                    .find(|(_, s)| s.id.as_deref() == Some(sc.source.as_str()))
            {
                let (at, rt) = crate::dsp::follower_coeffs(sc.attack, sc.release, sr as f32);
                links.push(DuckLink {
                    follower: ti,
                    source: si,
                    source_gain: source.gain,
                    gain_lane: LaneCursor::build(&source.automation, AutoTarget::Gain, source.gain),
                    env: 0.0,
                    at,
                    rt,
                    amount: sc.amount,
                });
            }
            layers.push(LayerAcc {
                id: layer_id,
                mute: t.mute,
                peak: 0.0,
                energy: 0.0,
            });
            out_tracks.push(TrackStream {
                voice,
                off,
                constant: pan_gains(t.pan.clamp(-1.0, 1.0), t.gain),
                gain: t.gain,
                pan: t.pan,
                gain_lane: LaneCursor::build(&t.automation, AutoTarget::Gain, t.gain),
                pan_lane: LaneCursor::build(&t.automation, AutoTarget::Pan, t.pan),
                dest: t.bus.as_deref().and_then(bus_index),
                bus_id: t.bus.clone(),
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
                Some(BusStream {
                    id: b.id.clone(),
                    chain: StereoChain::build(&b.effects, ctx)?,
                    gain: if b.gain.is_finite() { b.gain } else { 1.0 },
                })
            })
            .collect::<Option<_>>()?;
        let master = StereoChain::build(master, ctx)?;
        let n_tracks = out_tracks.len();
        let n_buses = buses.len();
        let block = || vec![0.0f32; ctx.cap];
        let capture = capture.then(|| Capture {
            layers,
            stems: stems.then(|| StemBufs {
                tracks: (0..n_tracks)
                    .map(|_| (vec![0.0; n], vec![0.0; n]))
                    .collect(),
                buses: (0..n_buses).map(|_| (vec![0.0; n], vec![0.0; n])).collect(),
            }),
        });
        Some(StreamTracks {
            tracks: out_tracks,
            links,
            buses,
            master,
            xs: (0..n_tracks).map(|_| block()).collect(),
            ducks: (0..n_tracks).map(|_| block()).collect(),
            bus_l: (0..n_buses).map(|_| block()).collect(),
            bus_r: (0..n_buses).map(|_| block()).collect(),
            sr,
            capture,
        })
    }

    /// Mix the bus samples `t0..t0 + out_l.len()` into `out_l`/`out_r`,
    /// advancing every carried state. `pitch` is the live pitch scale per
    /// sample (1.0 = as authored; the bounce).
    pub(crate) fn process(
        &mut self,
        out_l: &mut [f32],
        out_r: &mut [f32],
        t0: usize,
        pitch: &[f32],
    ) {
        let len = out_l.len();
        debug_assert!(out_r.len() == len && pitch.len() == len);
        let sr = self.sr;

        // 1. Track graphs, for the positioned part of the block.
        for (i, t) in self.tracks.iter_mut().enumerate() {
            let xs = &mut self.xs[i][..len];
            match &mut t.voice {
                Voice::Mono(src) if t0 + len > t.off => {
                    let skip = t.off.saturating_sub(t0);
                    xs[..skip].fill(0.0);
                    src.process(&mut xs[skip..], t0 + skip - t.off, &pitch[skip..]);
                }
                _ => xs.fill(0.0),
            }
        }
        // 2. Duck envelopes. A muted source feeds exact zeros (the envelope
        //    stays open) and its lane is never scanned; a stereo source steers
        //    the follower with its mid.
        for d in self.ducks.iter_mut() {
            d[..len].fill(1.0);
        }
        for link in self.links.iter_mut() {
            let src = &self.tracks[link.source];
            for j in 0..len {
                let p = t0 + j;
                let raw = if p < src.off {
                    None
                } else {
                    match &src.voice {
                        Voice::Muted => None,
                        Voice::Mono(_) => Some(self.xs[link.source][j]),
                        Voice::Stereo(l, r) => {
                            let k = p - src.off;
                            Some(0.5 * (l[k] + r[k]))
                        }
                    }
                };
                let sig = match raw {
                    Some(x) => {
                        let g = link
                            .gain_lane
                            .as_mut()
                            .map_or(link.source_gain, |c| c.at(p, sr));
                        x * g
                    }
                    None => 0.0,
                };
                let rect = sig.abs().min(1.0);
                let coeff = if rect > link.env { link.at } else { link.rt };
                link.env = rect + coeff * (link.env - rect);
                self.ducks[link.follower][j] = 1.0 - link.amount * link.env;
            }
        }
        // 3. The mix pass, in track declaration order.
        out_l.fill(0.0);
        out_r.fill(0.0);
        for b in self.bus_l.iter_mut().chain(self.bus_r.iter_mut()) {
            b[..len].fill(0.0);
        }
        for (i, t) in self.tracks.iter_mut().enumerate() {
            if matches!(t.voice, Voice::Muted) {
                continue;
            }
            for j in 0..len {
                let p = t0 + j;
                if p < t.off {
                    continue;
                }
                let (gl, gr) = match (&mut t.gain_lane, &mut t.pan_lane) {
                    (None, None) => t.constant,
                    (g, pn) => {
                        let gain = g.as_mut().map_or(t.gain, |c| c.at(p, sr));
                        let pan = pn.as_mut().map_or(t.pan, |c| c.at(p, sr)).clamp(-1.0, 1.0);
                        pan_gains(pan, gain)
                    }
                };
                let d = self.ducks[i][j];
                let (la, ra) = match &t.voice {
                    Voice::Mono(_) => {
                        let x = self.xs[i][j];
                        (x * gl * d, x * gr * d)
                    }
                    // A sampler track keeps its recorded stereo image; pan
                    // biases it.
                    Voice::Stereo(l, r) => {
                        let k = p - t.off;
                        (
                            l[k] * gl * std::f32::consts::SQRT_2 * d,
                            r[k] * gr * std::f32::consts::SQRT_2 * d,
                        )
                    }
                    Voice::Muted => unreachable!("muted tracks are skipped above"),
                };
                match t.dest {
                    None => {
                        out_l[j] += la;
                        out_r[j] += ra;
                    }
                    Some(bi) => {
                        self.bus_l[bi][j] += la;
                        self.bus_r[bi][j] += ra;
                    }
                }
                for (bi, amount) in &t.sends {
                    self.bus_l[*bi][j] += la * amount;
                    self.bus_r[*bi][j] += ra * amount;
                }
                if let Some(cap) = &mut self.capture {
                    let acc = &mut cap.layers[i];
                    acc.peak = acc.peak.max(la.abs()).max(ra.abs());
                    acc.energy += (la * la + ra * ra) as f64;
                    if let Some(stems) = &mut cap.stems {
                        stems.tracks[i].0[p] = la;
                        stems.tracks[i].1[p] = ra;
                    }
                }
            }
        }
        // 4. Buses, in declaration order: the insert chain, then the return
        //    fader onto the master bus.
        for (bi, b) in self.buses.iter_mut().enumerate() {
            let bl = &mut self.bus_l[bi][..len];
            let br = &mut self.bus_r[bi][..len];
            b.chain.process(bl, br);
            if let Some(stems) = self.capture.as_mut().and_then(|c| c.stems.as_mut()) {
                for j in 0..len {
                    stems.buses[bi].0[t0 + j] = bl[j] * b.gain;
                    stems.buses[bi].1[t0 + j] = br[j] * b.gain;
                }
            }
            for j in 0..len {
                out_l[j] += bl[j] * b.gain;
                out_r[j] += br[j] * b.gain;
            }
        }
        // 5. The master chain.
        self.master.process(out_l, out_r);
    }

    /// Sweep every filter's cutoff live, track graphs and insert chains
    /// alike (see [`StreamGraph::set_cutoff`](super::StreamGraph::set_cutoff)).
    pub(crate) fn set_cutoff(&mut self, scale: f32) {
        for t in self.tracks.iter_mut() {
            if let Voice::Mono(src) = &mut t.voice {
                src.set_cutoff(scale);
            }
        }
        for b in self.buses.iter_mut() {
            b.chain.set_cutoff(scale);
        }
        self.master.set_cutoff(scale);
    }

    /// The bounce's measurements once the whole document has been mixed:
    /// per-layer stats, and the stems when they were captured. `n` is the
    /// document length the RMS is taken over.
    fn finish(self, n: usize) -> (Vec<LayerStats>, Option<Vec<Stem>>) {
        let Some(cap) = self.capture else {
            return (Vec::new(), None);
        };
        let total: f64 = cap.layers.iter().map(|l| l.energy).sum();
        let layers = cap
            .layers
            .iter()
            .map(|l| {
                if l.mute {
                    return LayerStats {
                        id: l.id.clone(),
                        peak_dbfs: -180.0,
                        rms_dbfs: -180.0,
                        energy_pct: 0.0,
                        mute: true,
                    };
                }
                // RMS over the whole timeline (both channels), so layers
                // compare fairly regardless of where `at` placed them.
                let rms = ((l.energy / (2 * n) as f64) as f32).sqrt();
                LayerStats {
                    id: l.id.clone(),
                    peak_dbfs: crate::dsp::dbfs(l.peak),
                    rms_dbfs: crate::dsp::dbfs(rms),
                    energy_pct: if total > 0.0 {
                        ((l.energy / total) * 100.0) as f32
                    } else {
                        0.0
                    },
                    mute: false,
                }
            })
            .collect();
        let stems = cap.stems.map(|bufs| {
            let mut stems = Vec::with_capacity(bufs.tracks.len() + bufs.buses.len());
            for ((left, right), (t, l)) in bufs
                .tracks
                .into_iter()
                .zip(self.tracks.iter().zip(&cap.layers))
            {
                stems.push(Stem {
                    id: l.id.clone(),
                    is_bus: false,
                    bus: t.bus_id.clone(),
                    left,
                    right,
                });
            }
            for ((left, right), b) in bufs.buses.into_iter().zip(&self.buses) {
                stems.push(Stem {
                    id: format!("bus:{}", b.id),
                    is_bus: true,
                    bus: None,
                    left,
                    right,
                });
            }
            stems
        });
        (layers, stems)
    }
}

/// A bounced `tracks` document before its output stage: the stereo bus, the
/// per-layer stats, and the stems when requested.
pub(crate) struct OfflineMix {
    pub(crate) left: Vec<f32>,
    pub(crate) right: Vec<f32>,
    pub(crate) layers: Vec<LayerStats>,
    pub(crate) stems: Option<Vec<Stem>>,
}

/// Mix a `tracks` document in one block as long as itself. `None` for a
/// document whose root is not a mixer.
pub(crate) fn mix_offline(doc: &SoundDoc, want_stems: bool) -> Option<OfflineMix> {
    let ctx = Ctx::offline(doc.sample_rate, doc.frame_count());
    let mut mix = StreamTracks::build(doc, &ctx, true, want_stems)?;
    let n = ctx.n;
    let (mut left, mut right) = (vec![0.0f32; n], vec![0.0f32; n]);
    let pitch = vec![1.0f32; n];
    mix.process(&mut left, &mut right, 0, &pitch);
    let (layers, stems) = mix.finish(n);
    Some(OfflineMix {
        left,
        right,
        layers,
        stems,
    })
}
