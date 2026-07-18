//! The `tracks` mixer render: per-track evaluation onto the stereo bus with
//! equal-power panning, per-track RNG streams (schema v2), automation lanes,
//! per-layer contribution stats, and the master chain.

use super::effects::reverb;
use super::output::{make_loop_buffer, normalize_output, normalize_output_v4};
#[cfg(feature = "sampler")]
use super::seq::{SeqVoice, sampler_seq_stereo};
use super::{Signal, apply_processor, render_node};
use crate::dsl::{AutoLane, AutoTarget, Node, Playback, SeqWave, SoundDoc};
use crate::dsp::{Rng, layer_stream_key, peak_limit};

/// Equal-power channel gains for a `pan`/`gain` pair — one formula for the
/// constant fast path and the per-sample automated path, so they can never
/// drift (identical f32 op order, byte-identical output).
fn pan_gains(pan: f32, gain: f32) -> (f32, f32) {
    let theta = (pan + 1.0) * std::f32::consts::FRAC_PI_4;
    (theta.cos() * gain, theta.sin() * gain)
}

/// Derive a track's independent RNG stream from the document seed (schema
/// v2). `stream` is the track's FNV stream key (or `MASTER_STREAM`), not a
/// track index. SplitMix64 finalizer over a golden-gamma offset, so streams
/// never correlate with each other or with the v1 threaded stream.
fn track_stream_seed(seed: u64, stream: u64) -> u64 {
    crate::dsp::splitmix_mix(
        seed ^ stream
            .wrapping_add(1)
            .wrapping_mul(crate::dsp::GOLDEN_GAMMA),
    )
}

/// The master bus's stream key (validate rejects a layer id hashing to it).
const MASTER_STREAM: u64 = u64::MAX;

/// True when a track renders in native stereo (a sampler seq) — a cheap shape
/// test; the actual rendering happens in [`track_native_stereo`].
fn is_native_stereo(node: &Node) -> bool {
    matches!(
        node,
        Node::Seq {
            wave: SeqWave::Sampler,
            ..
        }
    )
}

/// Post-fader, pre-master snapshot of one layer's contribution to the stereo
/// bus — the balance numbers an agent mixes by. "Pre-master" matters: a master
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

/// Per-sample values for a track-automation `target`, or `None` if no lane
/// controls it (then the static value applies — the byte-identical fast path).
fn lane_for(
    automation: &[AutoLane],
    target: AutoTarget,
    n: usize,
    sr: u32,
    default: f32,
) -> Option<Vec<f32>> {
    let lane = automation.iter().find(|l| l.target == target)?;
    if lane.points.is_empty() {
        return Some(vec![default; n]);
    }
    let mut pts = lane.points.clone();
    pts.sort_by(|a, b| a.t.partial_cmp(&b.t).unwrap_or(std::cmp::Ordering::Equal));
    // Linear interpolation over the sorted breakpoints, holding flat past
    // either end. The sample time is strictly increasing, so a persistent
    // segment cursor replaces a from-zero scan per sample (O(n + p), not
    // O(n·p)). Strict `>` in the advance keeps the exact segment the scan
    // would pick — a sample landing on a breakpoint interpolates in the
    // earlier segment, so the floats (and the rendered bytes) are unchanged.
    let mut idx = 0;
    Some(
        (0..n)
            .map(|i| {
                let t = i as f32 / sr as f32;
                if t <= pts[0].t {
                    return pts[0].v;
                }
                let last = &pts[pts.len() - 1];
                if t >= last.t {
                    return last.v;
                }
                while t > pts[idx + 1].t {
                    idx += 1;
                }
                let (w0, w1) = (&pts[idx], &pts[idx + 1]);
                let span = (w1.t - w0.t).max(1e-9);
                w0.v + (w1.v - w0.v) * ((t - w0.t) / span)
            })
            .collect(),
    )
}

/// Render a `tracks` document to a finished stereo pair: each track is
/// rendered mono and equal-power panned onto the bus (sampler tracks keep
/// their native stereo), the master chain runs per channel (the reverb with
/// decorrelated tails), then loop/normalize apply jointly.
///
/// RNG model: schema v2 documents give every track (and the master bus) its
/// own deterministic stream, so editing, muting, or removing one track never
/// changes the noise content of its siblings. v1 documents keep the original
/// single stream threaded through the track list in order — their audio stays
/// byte-identical across upgrades.
pub fn render_tracks(doc: &SoundDoc) -> Option<TracksRender> {
    let Node::Tracks { tracks, master } = &doc.root else {
        return None;
    };
    let sr = doc.sample_rate;
    // validate() caps duration at 600 s; the clamp guards direct render calls
    // on unvalidated docs from an unbounded allocation (1e12 s ⇒ OOM abort).
    let n = ((doc.duration.clamp(0.0, 600.0) * sr as f32).ceil() as usize).max(1);
    let per_track_streams = doc.effective_version() >= 2;
    let engine = doc.effective_engine();
    let mut rng = Rng::new(doc.seed);
    let (mut left, mut right) = (vec![0.0f32; n], vec![0.0f32; n]);
    let mut layers = Vec::with_capacity(tracks.len());
    let mut energies = Vec::with_capacity(tracks.len());
    for (ti, t) in tracks.iter().enumerate() {
        let layer_id = t.id.clone().unwrap_or_else(|| format!("layer_{ti}"));
        // v2 streams are keyed by the stable layer id. The fallback hashes the
        // exact id `ensure_track_ids` will backfill, so a document's noise is
        // identical before and after the backfill pass.
        let stream = layer_stream_key(&layer_id);
        if t.mute {
            // Muted layers stay off the bus. v1's single stream must still
            // advance exactly as if the track had rendered, or muting one
            // layer would change every later layer's noise. (Cheap shape test:
            // native-stereo sampler tracks never touch the shared stream.)
            if !per_track_streams && !is_native_stereo(&t.node) {
                let _ = render_node(
                    &t.node,
                    n,
                    sr,
                    &mut rng,
                    engine,
                    track_stream_seed(doc.seed, stream),
                );
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
        // The layer lands `at` seconds into the song: render full-length, then
        // shift right and truncate (never shortening the render keeps RNG
        // consumption — and therefore v1 sibling content — offset-invariant).
        let off = ((t.at.max(0.0) * sr as f32).round() as usize).min(n);
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
        // Contribution stats accumulate over what actually lands on the bus
        // (post fader/pan/offset, pre master). Per-channel energy keeps them
        // pan-invariant: gl² + gr² = gain² for any pan.
        let (mut tpeak, mut tsum) = (0.0f32, 0.0f64);
        if let Some((l, r)) = track_native_stereo(&t.node, n, sr) {
            // A sampler track keeps its recorded stereo image; pan biases it.
            for i in 0..n - off {
                let (gl, gr) = gl_gr(i + off);
                let (la, ra) = (
                    l[i] * gl * std::f32::consts::SQRT_2,
                    r[i] * gr * std::f32::consts::SQRT_2,
                );
                left[i + off] += la;
                right[i + off] += ra;
                tpeak = tpeak.max(la.abs()).max(ra.abs());
                tsum += (la * la + ra * ra) as f64;
            }
        } else {
            let base = track_stream_seed(doc.seed, stream);
            let mono = if per_track_streams {
                let mut trng = Rng::new(base);
                render_node(&t.node, n, sr, &mut trng, engine, base)
            } else {
                render_node(&t.node, n, sr, &mut rng, engine, base)
            };
            for (i, x) in mono.into_iter().take(n - off).enumerate() {
                let (gl, gr) = gl_gr(i + off);
                let (la, ra) = (x * gl, x * gr);
                left[i + off] += la;
                right[i + off] += ra;
                tpeak = tpeak.max(la.abs()).max(ra.abs());
                tsum += (la * la + ra * ra) as f64;
            }
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
    if per_track_streams {
        rng = Rng::new(track_stream_seed(doc.seed, MASTER_STREAM));
    }
    // Master bus: run each processor on both channels with identical state
    // seeds (the rng is cloned so e.g. a duck trigger fires identically), and
    // give the reverb the classic Freeverb stereo spread for a wide tail.
    for m in master {
        if let Node::Reverb { room, mix } = m {
            left = reverb(&left, *room, *mix, sr, 0);
            right = reverb(&right, *room, *mix, sr, 23);
        } else {
            let mpath = track_stream_seed(doc.seed, MASTER_STREAM);
            let mut rl = rng.clone();
            left = apply_processor(m, &left, sr, &mut rl, engine, mpath);
            right = apply_processor(m, &right, sr, &mut rng, engine, mpath);
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
        if engine >= 4 {
            // One shared gain over the stereo program — the authored balance
            // is sacred. Engine ≤ 3 docs keep the original per-channel stage
            // bit-for-bit (it gain-matched L and R independently, collapsing
            // any asymmetric mix toward center).
            normalize_output_v4(&mut [&mut left, &mut right], nz, sr);
        } else {
            normalize_output(&mut left, nz);
            normalize_output(&mut right, nz);
        }
    }
    peak_limit(&mut [&mut left, &mut right]);
    Some(TracksRender {
        left,
        right,
        layers,
    })
}

/// A track whose node is directly a sampler seq renders in native stereo.
#[cfg(feature = "sampler")]
pub(super) fn track_native_stereo(node: &Node, n: usize, sr: u32) -> Option<(Signal, Signal)> {
    // Engine 0: unused by the sampler (external synth, engine-independent).
    let (voice, bpm, steps_per_beat, notes) = SeqVoice::from_node(node, 0)?;
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
