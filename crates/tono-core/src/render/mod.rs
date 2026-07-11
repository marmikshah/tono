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

pub(crate) use effects::{drive_antideriv, drive_curve};
pub(crate) use osc::{osc, poly_blep};
pub(crate) use seq::seq_to_signal;

use crate::dsl::{
    Adsr, AutoLane, AutoPoint, AutoTarget, Curve, Modulator, Node, Normalize, Playback, SeqWave,
    Shape, SoundDoc, Stereo, Value,
};
use crate::dsp::{
    Rng, db_to_lin, loudness_lufs, loudness_lufs_gated, peak_limit, true_peak,
    true_peak_oversampled,
};
use effects::{
    FilterKind, biquad, chorus, compress, drive_adaa, flanger, modal_bank, phaser, reverb,
};
use osc::{
    dust_signal, fm_signal, impact_signal, noise_signal, osc_signal, saw_signal, square_signal,
    super_signal, tri_signal,
};
#[cfg(feature = "sampler")]
use seq::{SeqVoice, sampler_seq_stereo};
use std::f32::consts::{FRAC_PI_2, TAU};

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

/// Derive track `i`'s independent RNG stream from the document seed (schema
/// v2). SplitMix64 finalizer over a golden-gamma offset, so streams never
/// correlate with each other or with the v1 threaded stream.
fn track_stream_seed(seed: u64, i: u64) -> u64 {
    crate::dsp::splitmix_mix(seed ^ i.wrapping_add(1).wrapping_mul(crate::dsp::GOLDEN_GAMMA))
}

/// The master bus's stream key (validate rejects a layer id hashing to it).
const MASTER_STREAM: u64 = u64::MAX;

use crate::dsp::{layer_stream_key, node_path, node_seed};

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
    Some(
        (0..n)
            .map(|i| eval_lane(&pts, i as f32 / sr as f32))
            .collect(),
    )
}

/// Linear interpolation over sorted breakpoints; holds flat past either end.
fn eval_lane(pts: &[AutoPoint], t: f32) -> f32 {
    let first = &pts[0];
    if t <= first.t {
        return first.v;
    }
    let last = &pts[pts.len() - 1];
    if t >= last.t {
        return last.v;
    }
    for w in pts.windows(2) {
        if t >= w[0].t && t <= w[1].t {
            let span = (w[1].t - w[0].t).max(1e-9);
            return w[0].v + (w[1].v - w[0].v) * ((t - w[0].t) / span);
        }
    }
    last.v
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
    let n = ((doc.duration * sr as f32).ceil() as usize).max(1);
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
        let theta = (t.pan.clamp(-1.0, 1.0) + 1.0) * std::f32::consts::FRAC_PI_4;
        let (glc, grc) = (theta.cos() * t.gain, theta.sin() * t.gain);
        let gain_lane = lane_for(&t.automation, AutoTarget::Gain, n, sr, t.gain);
        let pan_lane = lane_for(&t.automation, AutoTarget::Pan, n, sr, t.pan);
        let gl_gr = |pos: usize| -> (f32, f32) {
            match (&gain_lane, &pan_lane) {
                (None, None) => (glc, grc),
                (g, p) => {
                    let gain = g.as_ref().map_or(t.gain, |a| a[pos]);
                    let pan = p.as_ref().map_or(t.pan, |a| a[pos]).clamp(-1.0, 1.0);
                    let theta = (pan + 1.0) * std::f32::consts::FRAC_PI_4;
                    (theta.cos() * gain, theta.sin() * gain)
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
fn track_native_stereo(node: &Node, n: usize, sr: u32) -> Option<(Signal, Signal)> {
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
fn track_native_stereo(_node: &Node, _n: usize, _sr: u32) -> Option<(Signal, Signal)> {
    None
}

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

/// Extract the loop region `[start_secs, end_secs)` and equal-power crossfade
/// its last `crossfade_secs` onto its head, returning a buffer that repeats
/// seamlessly. The output length is the region minus the crossfade.
///
/// Overlap-loop: with region `r` of length `L` and crossfade `x`, the body is
/// `r[0..L-x]` with its first `x` samples replaced by a sin/cos blend of the
/// head (`r[i]`, fading in) and the tail (`r[L-x+i]`, fading out). The wrap
/// `out[last] → out[0]` then lands on adjacent original samples, so there is no
/// discontinuity.
pub fn make_loop_buffer(
    samples: &[f32],
    sr: u32,
    start_secs: f32,
    end_secs: Option<f32>,
    crossfade_secs: f32,
) -> Signal {
    let len = samples.len();
    let s = ((start_secs * sr as f32) as usize).min(len);
    let e = end_secs
        .map(|x| (x * sr as f32) as usize)
        .unwrap_or(len)
        .min(len);
    if e <= s {
        return samples.to_vec();
    }
    let region = &samples[s..e];
    let l = region.len();
    let x = ((crossfade_secs * sr as f32) as usize).min(l / 2);
    if x == 0 {
        return region.to_vec();
    }
    let out_len = l - x;
    let mut out = region[..out_len].to_vec();
    for (i, o) in out.iter_mut().take(x).enumerate() {
        let t = (i as f32 + 0.5) / x as f32;
        let fade_in = (FRAC_PI_2 * t).sin();
        let fade_out = (FRAC_PI_2 * t).cos();
        *o = region[i] * fade_in + region[out_len + i] * fade_out;
    }
    out
}

/// The loop-seam discontinuity in dB: the sample jump from the last sample back
/// to the first (lower ⇒ a cleaner seamless loop).
pub fn loop_seam_db(samples: &[f32]) -> f32 {
    if samples.len() < 2 {
        return -120.0;
    }
    let jump = (samples[0] - samples[samples.len() - 1]).abs();
    20.0 * jump.max(1e-9).log10()
}

/// Opt-in output stage: gain-match to a LUFS target (if given), soft-limiting
/// peaks into the `ceiling_dbtp` true-peak ceiling. Unlike a whole-buffer
/// attenuation, the soft-knee limiter only compresses the peaks, so dense /
/// peaky material (a BGM mix, layered impacts) actually REACHES the loudness
/// target instead of being dragged back down. Two measure→gain→limit passes
/// converge within ~1 dB.
fn normalize_output(samples: &mut [f32], nz: &Normalize) {
    let ceil = db_to_lin(nz.ceiling_dbtp);
    if let Some(target) = nz.target_lufs {
        for _ in 0..2 {
            let cur = loudness_lufs(samples);
            if cur <= -120.0 {
                break;
            }
            let g = db_to_lin(target - cur);
            for x in samples.iter_mut() {
                *x *= g;
            }
            soft_limit(samples, ceil);
        }
    }
    // Safety: catch inter-sample residue above the ceiling, then sample peak.
    true_peak_limit(samples, nz.ceiling_dbtp);
    peak_limit(&mut [samples]);
}

/// Engine ≥ 4 output stage over the whole program (1 = mono, 2 = stereo):
/// loudness is measured jointly with gated BS.1770 at the actual sample rate
/// and corrected with ONE shared gain (per-channel matching collapsed any
/// asymmetric mix toward center), and the ceiling is enforced against a real
/// oversampled true-peak estimate (the legacy linear estimate could never see
/// an inter-sample over, so the documented dBTP ceiling was not honored).
fn normalize_output_v4(channels: &mut [&mut [f32]], nz: &Normalize, sr: u32) {
    let ceil = db_to_lin(nz.ceiling_dbtp);
    if let Some(target) = nz.target_lufs {
        for _ in 0..2 {
            let cur = {
                let views: Vec<&[f32]> = channels.iter().map(|c| &**c).collect();
                loudness_lufs_gated(&views, sr)
            };
            if cur <= -120.0 {
                break;
            }
            let g = db_to_lin(target - cur);
            for c in channels.iter_mut() {
                for x in c.iter_mut() {
                    *x *= g;
                }
                soft_limit(c, ceil);
            }
        }
    }
    // Shared true-peak gain, then the joint sample-peak safety net.
    let tp = channels
        .iter()
        .map(|c| true_peak_oversampled(c))
        .fold(0.0f32, f32::max);
    if tp > ceil && tp > 0.0 {
        let g = ceil / tp;
        for c in channels.iter_mut() {
            for x in c.iter_mut() {
                *x *= g;
            }
        }
    }
    peak_limit(channels);
}

/// Soft-knee peak limiter: transparent below `0.7 × ceil`, smoothly (tanh)
/// compressed above, never exceeding `ceil`. C1-continuous at the knee.
fn soft_limit(samples: &mut [f32], ceil: f32) {
    const KNEE: f32 = 0.7;
    // A degenerate ceiling must not turn the mix into inf/NaN.
    let ceil = ceil.max(1e-9);
    for x in samples.iter_mut() {
        let v = *x / ceil;
        let a = v.abs();
        if a > KNEE {
            let compressed = KNEE + (1.0 - KNEE) * ((a - KNEE) / (1.0 - KNEE)).tanh();
            *x = v.signum() * compressed * ceil;
        }
    }
}

/// Scale so the estimated true peak sits at or below `ceiling_dbtp`. Pure
/// attenuation (never boosts), so it composes after loudness matching.
fn true_peak_limit(samples: &mut [f32], ceiling_dbtp: f32) {
    let ceil = db_to_lin(ceiling_dbtp);
    let tp = true_peak(samples);
    if tp > ceil && tp > 0.0 {
        let g = ceil / tp;
        for x in samples.iter_mut() {
            *x *= g;
        }
    }
}

/// Turn a finished mono render into a stereo (left, right) pair per the doc's
/// [`Stereo`] mode. Mono is the identity; Haas / Wide add width. The pair is
/// jointly peak-limited so widening never clips.
pub fn stereoize(mono: &[f32], stereo: Stereo, sr: u32) -> (Vec<f32>, Vec<f32>) {
    let (mut l, mut r) = match stereo {
        Stereo::Mono => (mono.to_vec(), mono.to_vec()),
        Stereo::Haas { ms, pan } => {
            let d = ((ms / 1000.0) * sr as f32) as usize;
            let delayed: Vec<f32> = (0..mono.len())
                .map(|i| if i >= d { mono[i - d] } else { 0.0 })
                .collect();
            // pan >= 0 → right leads (left is the delayed/trailing side).
            if pan >= 0.0 {
                (delayed, mono.to_vec())
            } else {
                (mono.to_vec(), delayed)
            }
        }
        Stereo::Wide { amount } => {
            let dec = allpass_decorrelate(mono, sr);
            let a = amount.clamp(0.0, 1.0);
            let mut l = Vec::with_capacity(mono.len());
            let mut r = Vec::with_capacity(mono.len());
            for i in 0..mono.len() {
                let mid = mono[i];
                let side = a * (mono[i] - dec[i]) * 0.5;
                l.push(mid + side);
                r.push(mid - side);
            }
            (l, r)
        }
    };
    peak_limit(&mut [&mut l, &mut r]);
    (l, r)
}

/// Decorrelate a mono signal with a short Schroeder all-pass chain (for the
/// `Wide` pseudo-stereo mode).
fn allpass_decorrelate(input: &[f32], sr: u32) -> Vec<f32> {
    let scale = sr as f32 / 44_100.0;
    let mut sig = input.to_vec();
    for &tune in &[225usize, 556, 441] {
        let len = ((tune as f32 * scale) as usize).max(1);
        let mut buf = vec![0.0f32; len];
        let mut idx = 0usize;
        let g = 0.7;
        for s in sig.iter_mut() {
            let buffered = buf[idx];
            let y = -*s * g + buffered;
            buf[idx] = *s + buffered * g;
            idx = (idx + 1) % len;
            *s = y;
        }
    }
    sig
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
