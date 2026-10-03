//! The `tracks` mixer render: per-track evaluation onto the stereo bus with
//! equal-power panning, per-track RNG streams, automation lanes,
//! per-layer contribution stats, and the master chain.

use super::effects::reverb;
use super::output::{make_loop_buffer, normalize_output};
#[cfg(feature = "sampler")]
use super::seq::{SeqVoice, sampler_seq_stereo};
use super::{Signal, apply_processor, render_node};
#[cfg(feature = "sampler")]
use crate::dsl::SeqWave;
use crate::dsl::{
    AutoCurve, AutoLane, AutoPoint, AutoTarget, Node, Playback, Sidechain, SoundDoc, Track,
};
use crate::dsp::{layer_stream_key, peak_limit};

/// One track's raw render, kept whole until the mix pass so sidechain
/// followers can read their source's signal regardless of declaration order.
enum TrackRender {
    /// Muted layers render nothing.
    Muted,
    /// A mono render plus its `at` offset in samples.
    Mono { off: usize, sig: Signal },
    /// A native-stereo (sampler) render plus its `at` offset in samples.
    Stereo {
        off: usize,
        left: Signal,
        right: Signal,
    },
}

/// Shared equal-power pan gains for offline and streaming mixing.
pub(crate) fn pan_gains(pan: f32, gain: f32) -> (f32, f32) {
    let theta = (pan + 1.0) * std::f32::consts::FRAC_PI_4;
    (crate::dsp::cos(theta) * gain, crate::dsp::sin(theta) * gain)
}

/// Derive a track's independent RNG stream from the document seed.
/// `stream` is the track's FNV stream key (or `MASTER_STREAM`), not a track
/// index. SplitMix64 finalizer over a golden-gamma offset. Shared
/// with the streaming mixer, which seeds each track's graph identically.
pub(crate) fn track_stream_seed(seed: u64, stream: u64) -> u64 {
    crate::dsp::splitmix_mix(
        seed ^ stream
            .wrapping_add(1)
            .wrapping_mul(crate::dsp::GOLDEN_GAMMA),
    )
}

/// The master bus's stream key (validate rejects a layer id hashing to it).
pub(crate) const MASTER_STREAM: u64 = u64::MAX;

/// Post-fader, pre-master snapshot of one layer's contribution to the stereo
/// bus — the balance numbers an author mixes by. "Pre-master" matters: a master
/// compressor / reverb reshapes the bus AFTER these are measured. Energy and
/// peak are measured per channel (pan-invariant: hard-panned and centered
/// layers of equal power read equal).
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize, schemars::JsonSchema)]
pub struct LayerStats {
    /// The layer's stable id.
    pub id: String,
    /// Peak of the layer's loudest bus channel in dBFS (−180 ⇒ silent/muted).
    pub peak_dbfs: f32,
    /// RMS of the layer's bus contribution over the WHOLE document timeline
    /// (per-channel energy, both channels), dBFS — comparable across layers
    /// regardless of their `at` placement.
    pub rms_dbfs: f32,
    /// Share of the summed pre-master layer energy, 0..100.
    pub energy_pct: f32,
    /// True when the layer is muted (it contributes nothing).
    pub mute: bool,
}

/// A finished mixer render: the stereo bus plus per-layer contribution stats
/// captured from the same pass (free — no extra render).
#[derive(Debug, PartialEq)]
pub struct TracksRender {
    /// The left channel of the mastered stereo bus.
    pub left: Signal,
    /// The right channel of the mastered stereo bus.
    pub right: Signal,
    /// Per-layer contribution stats captured from the same pass.
    pub layers: Vec<LayerStats>,
}

/// A persistent cursor over one automation lane, producing the exact values
/// the original whole-buffer scan produced — the ONE definition of the lane
/// math the offline mixer (per [`lane_for`]) and the streaming renderer's
/// block-wise evaluation share, so they can never drift. [`LaneCursor::at`]
/// must be called with monotonically non-decreasing sample indices (the
/// segment cursor only advances); starting mid-lane is fine — the strict-`>`
/// advance picks the same segment a from-zero scan would.
pub(crate) struct LaneCursor {
    /// Breakpoints sorted by time (the lane's authored order is not trusted).
    pts: Vec<AutoPoint>,
    curve: AutoCurve,
    /// The persistent segment cursor.
    idx: usize,
}

impl LaneCursor {
    /// The cursor for `target` in `automation`, or `None` if no lane controls
    /// it (then the static value applies — the byte-identical fast path).
    /// `default` is the static value an empty-points lane holds.
    pub(crate) fn build(automation: &[AutoLane], target: AutoTarget, default: f32) -> Option<Self> {
        let lane = automation.iter().find(|l| l.target == target)?;
        // An empty lane holds the static value; a single breakpoint holds
        // flat. Both collapse to one synthetic flat point so `at`'s guards
        // are total — a NaN sample time (or a NaN point time on an
        // unvalidated doc) would otherwise fall through both comparisons
        // into the segment scan and index out of bounds.
        if lane.points.len() < 2 {
            let v = lane.points.first().map_or(default, |p| p.v);
            return Some(LaneCursor {
                pts: vec![AutoPoint { t: 0.0, v }],
                curve: lane.curve,
                idx: 0,
            });
        }
        let mut pts = lane.points.clone();
        pts.sort_by(|a, b| a.t.partial_cmp(&b.t).unwrap_or(std::cmp::Ordering::Equal));
        Some(LaneCursor {
            pts,
            curve: lane.curve,
            idx: 0,
        })
    }

    /// The lane's value at sample `i` (`sr` the document's sample rate).
    /// Interpolation over the sorted breakpoints per the lane's curve,
    /// holding flat past either end. Strict `>` in the advance keeps the
    /// exact segment the from-zero scan would pick — a sample landing on a
    /// breakpoint interpolates in the earlier segment, preserving the
    /// rendered bytes.
    pub(crate) fn at(&mut self, i: usize, sr: u32) -> f32 {
        let t = i as f32 / sr as f32;
        // An unvalidated doc with sample_rate 0 makes frame 0 NaN (0.0/0.0),
        // which every comparison below rejects — hold the first point
        // instead of scanning off the end (infinite times already take the
        // `>= last.t` branch).
        if t.is_nan() {
            return self.pts[0].v;
        }
        if t <= self.pts[0].t {
            return self.pts[0].v;
        }
        let last = &self.pts[self.pts.len() - 1];
        if t >= last.t {
            return last.v;
        }
        while t > self.pts[self.idx + 1].t {
            self.idx += 1;
        }
        let (w0, w1) = (&self.pts[self.idx], &self.pts[self.idx + 1]);
        let span = (w1.t - w0.t).max(1e-9);
        let u = (t - w0.t) / span;
        match self.curve {
            AutoCurve::Linear => w0.v + (w1.v - w0.v) * u,
            // Hold w0 until the next breakpoint lands.
            AutoCurve::Step => {
                if u >= 1.0 {
                    w1.v
                } else {
                    w0.v
                }
            }
            // Exponential between same-sign positive endpoints; any other
            // segment degrades to linear (deterministic).
            AutoCurve::Exp => {
                if w0.v > 0.0 && w1.v > 0.0 {
                    w0.v * crate::dsp::powf(w1.v / w0.v, u)
                } else {
                    w0.v + (w1.v - w0.v) * u
                }
            }
        }
    }
}

/// Per-sample values for a track-automation `target`, or `None` if no lane
/// controls it (then the static value applies — the byte-identical fast path).
fn lane_for(
    automation: &[AutoLane],
    target: AutoTarget,
    n: usize,
    sr: u32,
    default: f32,
) -> Option<Vec<f32>> {
    let mut cursor = LaneCursor::build(automation, target, default)?;
    Some((0..n).map(|i| cursor.at(i, sr)).collect())
}

/// The gain-reduction envelope for one follower track: the `duck` node's
/// exact attack/release follower (same coefficients, same recurrence), so a
/// mixer-level pump matches the node-level one. It is driven by the source
/// track's positioned (post-`at`) mono signal scaled by the source's gain
/// fader — pre-pan, pre-master: the source as it actually lands on the bus,
/// so the follower ducks when the source sounds, not when its node starts.
/// A muted source is silence, so the envelope stays fully open.
fn duck_envelope(
    source: &TrackRender,
    source_track: &Track,
    sc: &Sidechain,
    n: usize,
    sr: u32,
) -> Vec<f32> {
    let mut sig = vec![0.0f32; n];
    let gain_lane = lane_for(
        &source_track.automation,
        AutoTarget::Gain,
        n,
        sr,
        source_track.gain,
    );
    let g = |pos: usize| gain_lane.as_ref().map_or(source_track.gain, |a| a[pos]);
    match source {
        TrackRender::Muted => {}
        TrackRender::Mono { off, sig: mono } => {
            for (i, x) in mono.iter().take(n - off).enumerate() {
                sig[i + off] = x * g(i + off);
            }
        }
        TrackRender::Stereo { off, left, right } => {
            // A native-stereo (sampler) source steers the follower with its
            // mid signal — the mono of the recorded image.
            for i in 0..n - off {
                sig[i + off] = 0.5 * (left[i] + right[i]) * g(i + off);
            }
        }
    }
    let srf = sr as f32;
    let at = crate::dsp::exp(-1.0 / (sc.attack.max(1e-4) * srf));
    let rt = crate::dsp::exp(-1.0 / (sc.release.max(1e-4) * srf));
    let mut env = 0.0f32;
    sig.into_iter()
        .map(|t| {
            let rect = t.abs().min(1.0);
            let coeff = if rect > env { at } else { rt };
            env = rect + coeff * (env - rect);
            1.0 - sc.amount * env
        })
        .collect()
}

/// Render the stereo mixing console, applying tracks, buses and master inserts.
/// Each named track owns an independent deterministic RNG stream; editing or
/// muting one track preserves its siblings. Loop and normalization apply jointly.
pub fn render_tracks(doc: &SoundDoc) -> Option<TracksRender> {
    render_tracks_impl(doc, false).map(|(r, _)| r)
}

/// One rendered stem: a track's positioned stereo contribution (post
/// fader/pan/offset/duck, pre bus/master) or a bus's processed return
/// (post inserts and return fader, pre master chain). `id` is the track's
/// layer id, or `bus:<id>` for a bus. Stems are pre-master-chain by
/// definition: the sum of every MASTER-routed track stem plus every bus
/// stem reproduces the mix the master chain hears — a bus-routed track's
/// stem is its channel output for your own processing, already included
/// in its bus's stem.
#[derive(Debug, Clone)]
pub struct Stem {
    /// The track's layer id, or `bus:<id>` for a bus stem.
    pub id: String,
    /// Whether this is a bus stem (a processed bus return).
    pub is_bus: bool,
    /// Where this track stem routes: `Some(bus id)` if its main output goes
    /// to a bus (the stem is then already inside that bus's stem), None if
    /// it lands on the master bus directly. Always None for bus stems.
    pub bus: Option<String>,
    /// The left channel.
    pub left: Signal,
    /// The right channel.
    pub right: Signal,
}

/// Render a `tracks` document to per-track and per-bus stereo stems (see
/// [`Stem`]). Muted tracks render as silent stems. `None` for a non-tracks
/// document, exactly like [`render_tracks`].
pub fn render_stems(doc: &SoundDoc) -> Option<Vec<Stem>> {
    let (_, stems) = render_tracks_impl(doc, true)?;
    Some(stems.expect("stems requested"))
}

fn render_tracks_impl(
    doc: &SoundDoc,
    want_stems: bool,
) -> Option<(TracksRender, Option<Vec<Stem>>)> {
    let Node::Tracks {
        tracks,
        master,
        buses,
    } = &doc.root
    else {
        return None;
    };
    let sr = doc.sample_rate;
    // validate() caps duration at 600 s; the clamp guards direct render calls
    // on unvalidated docs from an unbounded allocation (1e12 s ⇒ OOM abort).
    let n = ((doc.duration.clamp(0.0, 600.0) * sr as f32).ceil() as usize).max(1);
    let (mut left, mut right) = (vec![0.0f32; n], vec![0.0f32; n]);
    let mut layer_ids = Vec::with_capacity(tracks.len());
    let mut rendered = Vec::with_capacity(tracks.len());
    for (ti, t) in tracks.iter().enumerate() {
        let layer_id = t.id.clone().unwrap_or_else(|| format!("layer_{ti}"));
        // Streams are keyed by the stable layer id. The fallback hashes the
        // exact id `ensure_track_ids` will backfill, so a document's noise is
        // identical before and after the backfill pass.
        let stream = layer_stream_key(&layer_id);
        layer_ids.push(layer_id);
        if t.mute {
            rendered.push(TrackRender::Muted);
            continue;
        }
        let off = ((t.at.max(0.0) * sr as f32).round() as usize).min(n);
        if let Some((l, r)) = track_native_stereo(&t.node, n, sr) {
            rendered.push(TrackRender::Stereo {
                off,
                left: l,
                right: r,
            });
        } else {
            let base = track_stream_seed(doc.seed, stream);
            let mono = { render_node(&t.node, n, sr, base) };
            rendered.push(TrackRender::Mono { off, sig: mono });
        }
    }
    let mut layers = Vec::with_capacity(tracks.len());
    let mut energies = Vec::with_capacity(tracks.len());
    let mut stems = want_stems.then(Vec::new);
    let mut bus_bufs: Vec<(Vec<f32>, Vec<f32>)> = buses
        .iter()
        .map(|_| (vec![0.0f32; n], vec![0.0f32; n]))
        .collect();
    let bus_index = |id: &str| buses.iter().position(|b| b.id == id);
    let (mut cl, mut cr) = (vec![0.0f32; n], vec![0.0f32; n]);
    for (ti, t) in tracks.iter().enumerate() {
        let layer_id = layer_ids[ti].clone();
        if let TrackRender::Muted = &rendered[ti] {
            if let Some(stems) = &mut stems {
                stems.push(Stem {
                    id: layer_id.clone(),
                    is_bus: false,
                    bus: t.bus.clone(),
                    left: vec![0.0f32; n],
                    right: vec![0.0f32; n],
                });
            }
            layers.push(LayerStats {
                id: layer_id,
                peak_dbfs: -180.0,
                rms_dbfs: -180.0,
                energy_pct: 0.0,
                mute: true,
            });
            energies.push(0.0f64);
            continue;
        }
        // Equal-power pan/gain. With no automation this is constant (the proven
        // fast path, byte-identical); with automation it varies per bus sample.
        // The closure returns the same constant value when unautomated, so the
        // arithmetic on existing documents is unchanged.
        let (glc, grc) = pan_gains(t.pan.clamp(-1.0, 1.0), t.gain);
        let gain_lane = lane_for(&t.automation, AutoTarget::Gain, n, sr, t.gain);
        let pan_lane = lane_for(&t.automation, AutoTarget::Pan, n, sr, t.pan);
        let gl_gr = |pos: usize| -> (f32, f32) {
            match (&gain_lane, &pan_lane) {
                (None, None) => (glc, grc),
                (g, p) => {
                    let gain = g.as_ref().map_or(t.gain, |a| a[pos]);
                    let pan = p.as_ref().map_or(t.pan, |a| a[pos]).clamp(-1.0, 1.0);
                    pan_gains(pan, gain)
                }
            }
        };
        // The duck envelope follows the SOURCE track's signal (the source
        // itself renders untouched); this track's post-fader contribution is
        // multiplied by it. Unvalidated documents may name a missing source —
        // then there is no ducking (validate() rejects the document).
        let duck = t.sidechain.as_ref().and_then(|sc| {
            let (si, source) = tracks
                .iter()
                .enumerate()
                .find(|(_, s)| s.id.as_deref() == Some(sc.source.as_str()))?;
            Some(duck_envelope(&rendered[si], source, sc, n, sr))
        });
        // Contribution stats accumulate over what actually lands (post
        // fader/pan/offset/duck, pre bus/master). Per-channel energy keeps
        // them pan-invariant: gl² + gr² = gain² for any pan.
        let (mut tpeak, mut tsum) = (0.0f32, 0.0f64);
        cl.fill(0.0);
        cr.fill(0.0);
        let off = match &rendered[ti] {
            TrackRender::Muted => unreachable!("muted layers continue above"),
            TrackRender::Stereo { off, .. } | TrackRender::Mono { off, .. } => *off,
        };
        match &rendered[ti] {
            TrackRender::Muted => unreachable!("muted layers continue above"),
            TrackRender::Stereo {
                off,
                left: l,
                right: r,
            } => {
                // A sampler track keeps its recorded stereo image; pan biases it.
                for i in 0..n - off {
                    let (gl, gr) = gl_gr(i + off);
                    let d = duck.as_ref().map_or(1.0, |v| v[i + off]);
                    let (la, ra) = (
                        l[i] * gl * std::f32::consts::SQRT_2 * d,
                        r[i] * gr * std::f32::consts::SQRT_2 * d,
                    );
                    cl[i + off] = la;
                    cr[i + off] = ra;
                    tpeak = tpeak.max(la.abs()).max(ra.abs());
                    tsum += (la * la + ra * ra) as f64;
                }
            }
            TrackRender::Mono { off, sig } => {
                for (i, x) in sig.iter().take(n - off).enumerate() {
                    let (gl, gr) = gl_gr(i + off);
                    let d = duck.as_ref().map_or(1.0, |v| v[i + off]);
                    let (la, ra) = (x * gl * d, x * gr * d);
                    cl[i + off] = la;
                    cr[i + off] = ra;
                    tpeak = tpeak.max(la.abs()).max(ra.abs());
                    tsum += (la * la + ra * ra) as f64;
                }
            }
        }
        // Route the main output: the master bus, or the track's named bus.
        // (Only the contributed range is added — a full-range += 0.0 could
        // flip a −0.0 sample to +0.0 in slots this track never wrote.)
        let (dl, dr) = match t.bus.as_deref().and_then(bus_index) {
            None => (&mut left, &mut right),
            Some(bi) => {
                let (bl, br) = &mut bus_bufs[bi];
                (bl, br)
            }
        };
        for i in off..n {
            dl[i] += cl[i];
            dr[i] += cr[i];
        }
        // Post-fader sends: the same contribution, scaled, into each target.
        // (A track may send to the bus it's routed to — the sends simply add.)
        for s in &t.sends {
            let Some(bi) = bus_index(&s.bus) else {
                continue; // an unvalidated doc's dangling send is ignored
            };
            let amount = s.amount.clamp(0.0, 1.0);
            let (bl, br) = &mut bus_bufs[bi];
            for i in off..n {
                bl[i] += cl[i] * amount;
                br[i] += cr[i] * amount;
            }
        }
        // The stem is this exact contribution (pre bus/master).
        if let Some(stems) = &mut stems {
            stems.push(Stem {
                id: layer_id.clone(),
                is_bus: false,
                bus: t.bus.clone(),
                left: cl.clone(),
                right: cr.clone(),
            });
        }
        // RMS over the whole timeline (both channels), so layers compare
        // fairly regardless of where `at` placed them.
        let rms = ((tsum / (2 * n) as f64) as f32).sqrt();
        layers.push(LayerStats {
            id: layer_id,
            peak_dbfs: crate::dsp::dbfs(tpeak),
            rms_dbfs: crate::dsp::dbfs(rms),
            energy_pct: 0.0, // filled below once the total is known
            mute: false,
        });
        energies.push(tsum);
    }
    let total: f64 = energies.iter().sum();
    if total > 0.0 {
        for (l, e) in layers.iter_mut().zip(&energies) {
            l.energy_pct = ((e / total) * 100.0) as f32;
        }
    }
    for (bi, b) in buses.iter().enumerate() {
        let (mut bl, mut br) = std::mem::take(&mut bus_bufs[bi]);
        let bpath = track_stream_seed(doc.seed, layer_stream_key(&format!("bus:{}", b.id)));
        for fx in &b.effects {
            if let Node::Reverb { room, mix } = fx {
                bl = reverb(&bl, *room, *mix, sr, 0);
                br = reverb(&br, *room, *mix, sr, 23);
            } else {
                bl = apply_processor(fx, &bl, sr, bpath);
                br = apply_processor(fx, &br, sr, bpath);
            }
        }
        let gain = if b.gain.is_finite() { b.gain } else { 1.0 };
        if let Some(stems) = &mut stems {
            stems.push(Stem {
                id: format!("bus:{}", b.id),
                is_bus: true,
                bus: None,
                left: bl.iter().map(|x| x * gain).collect(),
                right: br.iter().map(|x| x * gain).collect(),
            });
        }
        for i in 0..n {
            left[i] += bl[i] * gain;
            right[i] += br[i] * gain;
        }
    }
    // Master bus: run each processor on both channels with identical state
    // seeds (so e.g. a duck trigger fires identically), and
    // give the reverb the classic Freeverb stereo spread for a wide tail.
    for m in master {
        if let Node::Reverb { room, mix } = m {
            left = reverb(&left, *room, *mix, sr, 0);
            right = reverb(&right, *room, *mix, sr, 23);
        } else {
            let mpath = track_stream_seed(doc.seed, MASTER_STREAM);

            left = apply_processor(m, &left, sr, mpath);
            right = apply_processor(m, &right, sr, mpath);
        }
    }
    if let Playback::Loop {
        start_secs,
        end_secs,
        crossfade_secs,
    } = doc.playback
    {
        left = make_loop_buffer(&left, sr, start_secs, end_secs, crossfade_secs);
        right = make_loop_buffer(&right, sr, start_secs, end_secs, crossfade_secs);
    }
    if let Some(nz) = &doc.normalize {
        normalize_output(&mut [&mut left, &mut right], nz, sr);
    }
    peak_limit(&mut [&mut left, &mut right]);
    Some((
        TracksRender {
            left,
            right,
            layers,
        },
        stems,
    ))
}

/// A track whose node is directly a sampler seq renders in native stereo.
#[cfg(feature = "sampler")]
pub(super) fn track_native_stereo(node: &Node, n: usize, sr: u32) -> Option<(Signal, Signal)> {
    let (voice, bpm, steps_per_beat, notes) = SeqVoice::from_node(node)?;
    if voice.wave != SeqWave::Sampler {
        return None;
    }
    let step_dur = sr as f32 * 60.0 / bpm / steps_per_beat.max(1) as f32;
    sampler_seq_stereo(&voice, notes, step_dur, n, sr)
}

/// Without the `sampler` feature there is no native-stereo SoundFont path.
#[cfg(not(feature = "sampler"))]
pub(super) fn track_native_stereo(_node: &Node, _n: usize, _sr: u32) -> Option<(Signal, Signal)> {
    None
}
