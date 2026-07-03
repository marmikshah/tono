//! Deterministic graph → samples renderer.
//!
//! Rendering is a pure function of `(graph, seed, sample_rate)`. Each node is
//! evaluated into a block of `f32` samples; combinators combine those blocks.
//! Processors transform the signal flowing through a `chain`.

use crate::dsl::{
    Adsr, AutoLane, AutoPoint, AutoTarget, Curve, DriveShape, KitStyle, Mode, Modulator, Node,
    NoiseColor, Normalize, Playback, SeqNote, SeqWave, Shape, SoundDoc, Stereo, SuperWave, Value,
};
use crate::dsp::{Rng, db_to_lin, loudness_lufs, peak_limit, true_peak};
use std::f32::consts::{FRAC_PI_2, LN_2, TAU};

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
    let mut z = seed ^ i.wrapping_add(1).wrapping_mul(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
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
    pub left: Signal,
    pub right: Signal,
    pub layers: Vec<LayerStats>,
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
        normalize_output(&mut left, nz);
        normalize_output(&mut right, nz);
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
    if let Node::Seq {
        bpm,
        steps_per_beat,
        wave: SeqWave::Sampler,
        duty,
        fm_ratio,
        fm_index,
        fm_strike,
        pluck_decay,
        pluck_body,
        pluck_pick,
        pluck_tone,
        piano_hammer,
        piano_strike,
        piano_inharm,
        piano_detune,
        piano_decay,
        kit,
        bass_cutoff,
        bass_env,
        bass_env_vel,
        bass_decay,
        bass_click,
        bass_body,
        bass_sub,
        bass_sub_ratio,
        bass_drive,
        bass_body_decay,
        sf2,
        sf2_preset,
        sf2_bank,
        swing,
        humanize,
        env,
        notes,
    } = node
    {
        let voice = SeqVoice {
            wave: SeqWave::Sampler,
            duty,
            fm_ratio: *fm_ratio,
            fm_index: *fm_index,
            fm_strike: *fm_strike,
            pluck_decay: *pluck_decay,
            pluck_body: *pluck_body,
            pluck_pick: *pluck_pick,
            pluck_tone: *pluck_tone,
            piano_hammer: *piano_hammer,
            piano_strike: *piano_strike,
            piano_inharm: *piano_inharm,
            piano_detune: *piano_detune,
            piano_decay: *piano_decay,
            kit: *kit,
            bass_cutoff: *bass_cutoff,
            bass_env: *bass_env,
            bass_env_vel: *bass_env_vel,
            bass_decay: *bass_decay,
            bass_click: *bass_click,
            bass_body: *bass_body,
            bass_sub: *bass_sub,
            bass_sub_ratio: *bass_sub_ratio,
            bass_drive: *bass_drive,
            bass_body_decay: *bass_body_decay,
            sf2,
            sf2_preset: *sf2_preset,
            sf2_bank: *sf2_bank,
            swing: *swing,
            humanize: *humanize,
            env,
            engine: 0, // unused by the sampler (external synth, engine-independent)
        };
        let step_dur = sr as f32 * 60.0 / bpm / (*steps_per_beat).max(1) as f32;
        return sampler_seq_stereo(&voice, notes, step_dur, n, sr);
    }
    None
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

/// Soft-knee peak limiter: transparent below `0.7 × ceil`, smoothly (tanh)
/// compressed above, never exceeding `ceil`. C1-continuous at the knee.
fn soft_limit(samples: &mut [f32], ceil: f32) {
    const KNEE: f32 = 0.7;
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
                let p = (t / secs).clamp(0.0, 1.0);
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
        Value::Modulated(Modulator::Arp { steps, rate }) => (0..n)
            .map(|i| {
                let t = i as f32 / srf;
                let idx = (t * rate) as usize % steps.len();
                steps[idx]
            })
            .collect(),
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
    let mut h = seed ^ 0x9E37_79B9_7F4A_7C15;
    for bits in [from.to_bits(), to.to_bits(), rate.to_bits()] {
        h = (h ^ bits as u64).wrapping_mul(0x0000_0100_0000_01B3);
    }
    h
}

/// Sparse stochastic impulses — a Poisson click train smoothed by a one-pole
/// decay so overlapping grains sum. `density` events/sec each fire with random
/// ± amplitude; `decay` sets the per-grain ring (0 = bare impulses). Draws from
/// the render stream (like [`noise_signal`]).
fn dust_signal(density: f32, decay: f32, n: usize, sr: u32, rng: &mut Rng) -> Signal {
    let srf = sr as f32;
    let p = (density / srf).clamp(0.0, 1.0); // event probability per sample
    let g = if decay > 0.0 {
        (-1.0 / (decay * srf)).exp()
    } else {
        0.0
    };
    let mut y = 0.0f32;
    (0..n)
        .map(|_| {
            let imp = if rng.unit() < p { rng.bi() } else { 0.0 };
            y = imp + g * y;
            y
        })
        .collect()
}

/// PolyBLEP residual for band-limited oscillators: corrects the discontinuity
/// at a phase edge to suppress aliasing. `t` is the phase (0..1), `dt` the
/// per-sample phase increment.
pub(crate) fn poly_blep(mut t: f32, dt: f32) -> f32 {
    if dt <= 0.0 {
        return 0.0;
    }
    if t < dt {
        t /= dt;
        t + t - t * t - 1.0
    } else if t > 1.0 - dt {
        t = (t - 1.0) / dt;
        t * t + t + t + 1.0
    } else {
        0.0
    }
}

/// Unit-amplitude oscillator value in [-1, 1] for a phase in [0, 1).
pub(crate) fn osc(shape: Shape, phase: f32) -> f32 {
    match shape {
        Shape::Sine => (TAU * phase).sin(),
        Shape::Square => {
            if phase < 0.5 {
                1.0
            } else {
                -1.0
            }
        }
        Shape::Triangle => {
            if phase < 0.5 {
                4.0 * phase - 1.0
            } else {
                3.0 - 4.0 * phase
            }
        }
        Shape::Saw => 2.0 * phase - 1.0,
    }
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

#[derive(Clone, Copy)]
enum FilterKind {
    Low,
    High,
    Band,
    Notch,
    /// Peaking EQ, gain in dB.
    Peak(f32),
    /// Low shelf, gain in dB.
    LowShelf(f32),
    /// High shelf, gain in dB.
    HighShelf(f32),
}

/// RBJ biquad with per-sample coefficient updates so the cutoff can be
/// modulated. State carried in Direct Form I. Peaking/shelving kinds carry a
/// dB gain (`A = 10^(gain/40)`).
fn biquad(input: &[f32], cutoff: &Value, q: f32, sr: u32, kind: FilterKind) -> Signal {
    let fc = eval_value(cutoff, input.len(), sr);
    let srf = sr as f32;
    let q = q.max(0.05);
    let nyq = srf / 2.0;
    let amp = match kind {
        FilterKind::Peak(g) | FilterKind::LowShelf(g) | FilterKind::HighShelf(g) => {
            10f32.powf(g / 40.0)
        }
        _ => 1.0,
    };
    let (mut x1, mut x2, mut y1, mut y2) = (0.0f32, 0.0f32, 0.0f32, 0.0f32);
    let mut out = Vec::with_capacity(input.len());
    for (i, &x0) in input.iter().enumerate() {
        let f = fc[i].clamp(20.0, nyq - 100.0);
        let w0 = TAU * f / srf;
        let (sin, cos) = w0.sin_cos();
        let alpha = sin / (2.0 * q);
        let (b0, b1, b2, a0, a1, a2) = match kind {
            FilterKind::Low => (
                (1.0 - cos) / 2.0,
                1.0 - cos,
                (1.0 - cos) / 2.0,
                1.0 + alpha,
                -2.0 * cos,
                1.0 - alpha,
            ),
            FilterKind::High => (
                (1.0 + cos) / 2.0,
                -(1.0 + cos),
                (1.0 + cos) / 2.0,
                1.0 + alpha,
                -2.0 * cos,
                1.0 - alpha,
            ),
            FilterKind::Band => (alpha, 0.0, -alpha, 1.0 + alpha, -2.0 * cos, 1.0 - alpha),
            FilterKind::Notch => (1.0, -2.0 * cos, 1.0, 1.0 + alpha, -2.0 * cos, 1.0 - alpha),
            FilterKind::Peak(_) => (
                1.0 + alpha * amp,
                -2.0 * cos,
                1.0 - alpha * amp,
                1.0 + alpha / amp,
                -2.0 * cos,
                1.0 - alpha / amp,
            ),
            FilterKind::LowShelf(_) => {
                let s = 2.0 * amp.sqrt() * alpha;
                let (ap1, am1) = (amp + 1.0, amp - 1.0);
                (
                    amp * (ap1 - am1 * cos + s),
                    2.0 * amp * (am1 - ap1 * cos),
                    amp * (ap1 - am1 * cos - s),
                    ap1 + am1 * cos + s,
                    -2.0 * (am1 + ap1 * cos),
                    ap1 + am1 * cos - s,
                )
            }
            FilterKind::HighShelf(_) => {
                let s = 2.0 * amp.sqrt() * alpha;
                let (ap1, am1) = (amp + 1.0, amp - 1.0);
                (
                    amp * (ap1 + am1 * cos + s),
                    -2.0 * amp * (am1 + ap1 * cos),
                    amp * (ap1 + am1 * cos - s),
                    ap1 - am1 * cos + s,
                    2.0 * (am1 - ap1 * cos),
                    ap1 - am1 * cos - s,
                )
            }
        };
        let y0 = (b0 / a0) * x0 + (b1 / a0) * x1 + (b2 / a0) * x2 - (a1 / a0) * y1 - (a2 / a0) * y2;
        x2 = x1;
        x1 = x0;
        y2 = y1;
        y1 = y0;
        out.push(y0);
    }
    out
}

/// Schroeder reverb: parallel feedback combs into series allpasses. Tunings are
/// the classic Freeverb values, scaled to the sample rate.
fn reverb(input: &[f32], room: f32, mix: f32, sr: u32, spread: usize) -> Signal {
    let scale = sr as f32 / 44_100.0;
    let comb_tunings = [
        1116 + spread,
        1188 + spread,
        1277 + spread,
        1356 + spread,
        1422 + spread,
        1491 + spread,
    ];
    let allpass_tunings = [556 + spread, 441 + spread, 341 + spread, 225 + spread];
    let feedback = 0.7 + 0.28 * room.clamp(0.0, 1.0);
    let damp = 0.2;

    let mut wet = vec![0.0f32; input.len()];
    // Parallel combs (summed).
    for &tune in &comb_tunings {
        let len = ((tune as f32 * scale) as usize).max(1);
        let mut buf = vec![0.0f32; len];
        let mut idx = 0usize;
        let mut filter_store = 0.0f32;
        for (i, &x) in input.iter().enumerate() {
            let y = buf[idx];
            filter_store = y * (1.0 - damp) + filter_store * damp;
            buf[idx] = x + filter_store * feedback;
            idx = (idx + 1) % len;
            wet[i] += y;
        }
    }
    // Series allpasses.
    for &tune in &allpass_tunings {
        let len = ((tune as f32 * scale) as usize).max(1);
        let mut buf = vec![0.0f32; len];
        let mut idx = 0usize;
        let g = 0.5;
        for w in wet.iter_mut() {
            let buffered = buf[idx];
            let y = -*w + buffered;
            buf[idx] = *w + buffered * g;
            idx = (idx + 1) % len;
            *w = y;
        }
    }
    let mix = mix.clamp(0.0, 1.0);
    let comb_norm = 1.0 / comb_tunings.len() as f32;
    input
        .iter()
        .zip(wet)
        .map(|(dry, w)| dry * (1.0 - mix) + (w * comb_norm) * mix)
        .collect()
}

/// Apply a waveshaper curve to a single sample.
pub(crate) fn drive_curve(x: f32, shape: DriveShape) -> f32 {
    match shape {
        DriveShape::Tanh => x.tanh(),
        DriveShape::Hard => x.clamp(-1.0, 1.0),
        DriveShape::Fold => {
            // Reflect anything outside [-1, 1] back inward (wavefolding).
            let mut y = x;
            while !(-1.0..=1.0).contains(&y) {
                if y > 1.0 {
                    y = 2.0 - y;
                } else {
                    y = -2.0 - y;
                }
            }
            y
        }
    }
}

/// Antiderivative F(x) of each waveshaper, used by [`drive_adaa`]. F'(x) =
/// `drive_curve(x, shape)`. The additive constant is irrelevant — ADAA only
/// ever uses differences `F(x1) − F(x0)`.
pub(crate) fn drive_antideriv(x: f32, shape: DriveShape) -> f32 {
    match shape {
        // ∫ tanh = ln(cosh x). Computed as |x| + ln(1+e^{−2|x|}) − ln 2 so it
        // never overflows for large |x| (cosh would).
        DriveShape::Tanh => {
            let a = x.abs();
            a + (-2.0 * a).exp().ln_1p() - LN_2
        }
        // ∫ clamp(x,−1,1): x²/2 inside the linear region, |x|−1/2 outside
        // (continuous at ±1, both give 1/2).
        DriveShape::Hard => {
            let a = x.abs();
            if a <= 1.0 { 0.5 * x * x } else { a - 0.5 }
        }
        // The fold is a period-4 triangle wave; its antiderivative is the
        // continuous, period-4 piecewise parabola below (zero-mean ⇒ bounded,
        // so it is safe for arbitrarily large |x|). Reduce x into one period
        // first: p = (x+1) mod 4 ∈ [0,4).
        DriveShape::Fold => {
            let p = (x + 1.0).rem_euclid(4.0);
            if p <= 2.0 {
                0.5 * (p - 1.0) * (p - 1.0)
            } else {
                1.0 - 0.5 * (p - 3.0) * (p - 3.0)
            }
        }
    }
}

/// First-order antiderivative anti-aliasing for the memoryless waveshaper.
///
/// A pointwise nonlinearity sprays harmonics past Nyquist that fold back as
/// inharmonic "digital" grit. ADAA replaces `f(x)` with the average of `f`
/// over `[x[n-1], x[n]]` — `(F(x[n]) − F(x[n-1])) / (x[n] − x[n-1])` — which
/// band-limits the result, suppressing the foldback. The `f(midpoint)`
/// fallback avoids the 0/0 (and its catastrophic cancellation) when
/// consecutive inputs are nearly equal. One sample of state is carried across
/// the block. A one-pole DC blocker follows: the difference-quotient leaves a
/// small DC term on asymmetric input.
fn drive_adaa(input: &[f32], amount: &[f32], shape: DriveShape) -> Signal {
    const EPS: f32 = 1e-5;
    // ~5 Hz one-pole DC blocker (y[n] = x[n] − x[n−1] + R·y[n−1]).
    const R: f32 = 0.9995;
    let mut x_prev = 0.0f32;
    let mut f_prev = drive_antideriv(0.0, shape);
    let (mut dc_x, mut dc_y) = (0.0f32, 0.0f32);
    let mut out = Vec::with_capacity(input.len());
    for (&x, &amt) in input.iter().zip(amount) {
        let xn = amt.max(0.0) * x;
        let f = drive_antideriv(xn, shape);
        let d = xn - x_prev;
        let y = if d.abs() > EPS {
            (f - f_prev) / d
        } else {
            drive_curve(0.5 * (xn + x_prev), shape)
        };
        x_prev = xn;
        f_prev = f;
        let yb = y - dc_x + R * dc_y;
        dc_x = y;
        dc_y = yb;
        out.push(yb);
    }
    out
}

/// Impact exciter: a single raised-cosine (Hann-lobe) force pulse at the start
/// of the buffer, the rest silence. `hardness` sets the contact time — a hard
/// strike is a brief, wide-band click (lights up high modes); a soft strike is
/// a longer, duller bump. The pulse is normalised to UNIT AREA (× `velocity`),
/// so `velocity` sets the total impulse delivered and `hardness` only shapes
/// its spectrum — a downstream [`Node::Modal`] bank then rings to a level set
/// by its modes' `gain`, independent of the strike width.
fn impact_signal(hardness: f32, velocity: f32, n: usize, sr: u32) -> Signal {
    let h = hardness.clamp(0.0, 1.0);
    let v = velocity.clamp(0.0, 1.0);
    // Contact time: soft ≈ 8 ms, hard ≈ 0.3 ms.
    let width_s = 0.008 * (1.0 - h) + 0.0003 * h;
    let w = ((width_s * sr as f32).round() as usize).max(1);
    // A Hann lobe sums to ≈ w/2; normalise so the whole pulse has area `v`.
    let norm = v / (0.5 * w as f32);
    let mut out = vec![0.0f32; n];
    for (i, o) in out.iter_mut().enumerate().take(w.min(n)) {
        let phase = (i as f32 + 0.5) / w as f32;
        *o = norm * 0.5 * (1.0 - (TAU * phase).cos());
    }
    out
}

/// Modal resonator bank: sum of N parallel two-pole resonators driven by the
/// incoming signal. Each mode is a complex-conjugate pole pair at radius `r`
/// and angle `ω`: `y[n] = b0·x[n] + 2r·cos(ω)·y[n-1] − r²·y[n-2]`. The pole
/// radius sets the decay exactly — `r^(decay·sr) = 0.001`, i.e. −60 dB at the
/// mode's ring time — and `b0 = gain·sin(ω)` normalises the impulse-response
/// peak to `gain`, so a mode's loudness is its `gain` regardless of how long
/// it rings. Coefficients are constant per mode (LTI), so no per-sample
/// recompute and no zipper. Deterministic: pure f32 arithmetic, fixed
/// coefficients.
fn modal_bank(input: &[f32], modes: &[Mode], mix: f32, sr: u32) -> Signal {
    let srf = sr as f32;
    let nyq = srf * 0.5;
    let mix = mix.clamp(0.0, 1.0);
    let mut wet = vec![0.0f32; input.len()];
    for m in modes {
        let f0 = m.freq.clamp(1.0, nyq - 1.0);
        let decay = m.decay.max(1e-3);
        let w0 = TAU * f0 / srf;
        let (sin0, cos0) = (w0.sin(), w0.cos());
        // r so the ring reaches −60 dB (×0.001) after `decay` seconds.
        let r = (-6.907_755 / (decay * srf)).exp();
        let a1 = 2.0 * r * cos0;
        let a2 = -r * r;
        let b0 = m.gain * sin0; // impulse-response peak ≈ gain
        let (mut y1, mut y2) = (0.0f32, 0.0f32);
        for (o, &x) in wet.iter_mut().zip(input) {
            let y = b0 * x + a1 * y1 + a2 * y2;
            y2 = y1;
            y1 = y;
            *o += y;
        }
    }
    input
        .iter()
        .zip(wet)
        .map(|(d, w)| d * (1.0 - mix) + w * mix)
        .collect()
}

/// Chorus: a single voice of modulated delay mixed with the dry signal.
fn chorus(input: &[f32], rate: f32, depth: f32, mix: f32, sr: u32) -> Signal {
    let srf = sr as f32;
    let base = 0.015 * srf; // ~15 ms centre delay
    let swing = depth.clamp(0.0, 1.0) * 0.010 * srf; // up to ±10 ms
    let max_delay = (base + swing) as usize + 2;
    let mut buf = vec![0.0f32; max_delay];
    let mut w = 0usize;
    let mix = mix.clamp(0.0, 1.0);
    let mut out = Vec::with_capacity(input.len());
    for (i, &x) in input.iter().enumerate() {
        buf[w] = x;
        let lfo = (TAU * rate * i as f32 / srf).sin();
        let delay = base + swing * lfo;
        // Fractional read via linear interpolation.
        let read = w as f32 - delay;
        let read = read.rem_euclid(max_delay as f32);
        let i0 = read.floor() as usize % max_delay;
        let i1 = (i0 + 1) % max_delay;
        let frac = read - read.floor();
        let wet = buf[i0] * (1.0 - frac) + buf[i1] * frac;
        out.push(x * (1.0 - mix) + wet * mix);
        w = (w + 1) % max_delay;
    }
    out
}

/// Flanger: a 0.5–6 ms swept delay with feedback, mixed against the dry path.
fn flanger(input: &[f32], rate: f32, depth: f32, feedback: f32, mix: f32, sr: u32) -> Signal {
    let srf = sr as f32;
    let base = 0.0025 * srf; // 2.5 ms centre
    let swing = depth.clamp(0.0, 1.0) * 0.002 * srf; // up to ±2 ms
    let max_delay = (base + swing) as usize + 2;
    let mut buf = vec![0.0f32; max_delay];
    let mut w = 0usize;
    let fb = feedback.clamp(0.0, 0.95);
    let mix = mix.clamp(0.0, 1.0);
    let mut out = Vec::with_capacity(input.len());
    for (i, &x) in input.iter().enumerate() {
        let lfo = (TAU * rate * i as f32 / srf).sin();
        let delay = base + swing * lfo;
        let read = (w as f32 - delay).rem_euclid(max_delay as f32);
        let i0 = read.floor() as usize % max_delay;
        let i1 = (i0 + 1) % max_delay;
        let frac = read - read.floor();
        let wet = buf[i0] * (1.0 - frac) + buf[i1] * frac;
        buf[w] = x + wet * fb;
        w = (w + 1) % max_delay;
        out.push(x * (1.0 - mix) + wet * mix);
    }
    out
}

/// Phaser: four first-order all-pass stages with an LFO-swept coefficient and
/// feedback — swept spectral notches.
fn phaser(input: &[f32], rate: f32, depth: f32, feedback: f32, mix: f32, sr: u32) -> Signal {
    let srf = sr as f32;
    let fb = feedback.clamp(0.0, 0.95);
    let mix = mix.clamp(0.0, 1.0);
    let depth = depth.clamp(0.0, 1.0);
    let mut x1 = [0.0f32; 4];
    let mut y1 = [0.0f32; 4];
    let mut last_wet = 0.0f32;
    let mut out = Vec::with_capacity(input.len());
    for (i, &x) in input.iter().enumerate() {
        // Sweep the all-pass coefficient between ~0.15 and ~0.85.
        let lfo = 0.5 + 0.5 * (TAU * rate * i as f32 / srf).sin();
        let g = 0.15 + 0.7 * depth * lfo;
        let mut s = x + last_wet * fb;
        for k in 0..4 {
            let y = -g * s + x1[k] + g * y1[k];
            x1[k] = s;
            y1[k] = y;
            s = y;
        }
        last_wet = s;
        out.push(x * (1.0 - mix) + s * mix);
    }
    out
}

/// Feed-forward compressor with a peak-detector envelope follower.
fn compress(
    input: &[f32],
    threshold_db: f32,
    ratio: f32,
    attack: f32,
    release: f32,
    makeup_db: f32,
    sr: u32,
) -> Signal {
    let srf = sr as f32;
    let at = (-1.0 / (attack.max(1e-4) * srf)).exp();
    let rt = (-1.0 / (release.max(1e-4) * srf)).exp();
    let makeup = 10f32.powf(makeup_db / 20.0);
    let ratio = ratio.max(1.0);
    let mut env = 0.0f32; // envelope in linear amplitude
    let mut out = Vec::with_capacity(input.len());
    for &x in input {
        let rect = x.abs();
        // Attack when rising, release when falling.
        let coeff = if rect > env { at } else { rt };
        env = rect + coeff * (env - rect);
        let env_db = 20.0 * env.max(1e-9).log10();
        let gain_db = if env_db > threshold_db {
            -(env_db - threshold_db) * (1.0 - 1.0 / ratio)
        } else {
            0.0
        };
        let g = 10f32.powf(gain_db / 20.0);
        out.push(x * g * makeup);
    }
    out
}

/// Drive a phase accumulator at a (possibly modulated) frequency and map each
/// phase to a sample via `wave`.
fn osc_signal(freq: &Value, n: usize, sr: u32, wave: impl Fn(f32) -> f32) -> Signal {
    let f = eval_value(freq, n, sr);
    let srf = sr as f32;
    let mut phase = 0.0f32;
    let mut out = Vec::with_capacity(n);
    for &fi in f.iter() {
        out.push(wave(phase));
        phase += fi.max(0.0) / srf;
        phase -= phase.floor();
    }
    out
}

/// Band-limited square / pulse with a per-sample (modulatable) duty — PWM.
/// PolyBLEP corrects both the rising (phase 0) and falling (phase = duty) edges.
fn square_signal(freq: &Value, duty: &Value, n: usize, sr: u32) -> Signal {
    let f = eval_value(freq, n, sr);
    let d = eval_value(duty, n, sr);
    let srf = sr as f32;
    let mut phase = 0.0f32;
    let mut out = Vec::with_capacity(n);
    for i in 0..n {
        let duty = d[i].clamp(0.01, 0.99);
        let dt = f[i].max(0.0) / srf;
        let mut v = if phase < duty { 1.0 } else { -1.0 };
        v += poly_blep(phase, dt);
        v -= poly_blep((phase - duty + 1.0).fract(), dt);
        out.push(v);
        phase += dt;
        phase -= phase.floor();
    }
    out
}

/// Band-limited sawtooth (naive ramp minus a PolyBLEP at the wrap).
fn saw_signal(freq: &Value, n: usize, sr: u32) -> Signal {
    let f = eval_value(freq, n, sr);
    let srf = sr as f32;
    let mut phase = 0.0f32;
    let mut out = Vec::with_capacity(n);
    for &fi in f.iter() {
        let dt = fi.max(0.0) / srf;
        out.push((2.0 * phase - 1.0) - poly_blep(phase, dt));
        phase += dt;
        phase -= phase.floor();
    }
    out
}

/// Band-limited triangle: integrate a band-limited (PolyBLEP) square. A leaky
/// integrator removes DC drift. Clean at high pitch, unlike a naive triangle.
fn tri_signal(freq: &Value, n: usize, sr: u32) -> Signal {
    let f = eval_value(freq, n, sr);
    let srf = sr as f32;
    let mut phase = 0.0f32;
    let mut tri = 0.0f32;
    let mut out = Vec::with_capacity(n);
    for &fi in f.iter() {
        let dt = fi.max(0.0) / srf;
        // Band-limited square (duty 0.5): rising edge at 0, falling at 0.5.
        let mut sq = if phase < 0.5 { 1.0 } else { -1.0 };
        sq += poly_blep(phase, dt);
        sq -= poly_blep((phase + 0.5).fract(), dt);
        // Integrate (slope ±4/period ⇒ unit-amplitude triangle); leak out DC.
        tri = tri * 0.9995 + 4.0 * dt * sq;
        out.push(tri);
        phase += dt;
        phase -= phase.floor();
    }
    out
}

/// Unison super-oscillator: sum `voices` detuned band-limited saw/square copies,
/// phase-spread for width, scaled by 1/voices so the level stays bounded.
fn super_signal(
    wave: SuperWave,
    freq: &Value,
    voices: u32,
    detune_cents: f32,
    n: usize,
    sr: u32,
) -> Signal {
    let f = eval_value(freq, n, sr);
    let srf = sr as f32;
    let v = voices.clamp(1, 16);
    let mut out = vec![0.0f32; n];
    for k in 0..v {
        // Symmetric detune spread across [-detune, +detune] cents.
        let cents = if v == 1 {
            0.0
        } else {
            -detune_cents + 2.0 * detune_cents * (k as f32 / (v as f32 - 1.0))
        };
        let ratio = 2f32.powf(cents / 1200.0);
        let mut phase = k as f32 / v as f32; // decorrelate voice phases
        for (i, o) in out.iter_mut().enumerate() {
            let dt = (f[i].max(0.0) * ratio) / srf;
            let s = match wave {
                SuperWave::Sawtooth => (2.0 * phase - 1.0) - poly_blep(phase, dt),
                SuperWave::Square => {
                    let mut sq = if phase < 0.5 { 1.0 } else { -1.0 };
                    sq += poly_blep(phase, dt);
                    sq -= poly_blep((phase + 0.5).fract(), dt);
                    sq
                }
            };
            *o += s;
            phase += dt;
            phase -= phase.floor();
        }
    }
    let scale = 1.0 / v as f32;
    for o in out.iter_mut() {
        *o *= scale;
    }
    out
}

/// Generate `n` samples of coloured noise.
fn noise_signal(color: NoiseColor, n: usize, rng: &mut Rng) -> Signal {
    match color {
        NoiseColor::White => (0..n).map(|_| rng.bi()).collect(),
        NoiseColor::Pink => {
            // Paul Kellet's economical pink-noise filter.
            let (mut b0, mut b1, mut b2, mut b3, mut b4, mut b5, mut b6) =
                (0.0f32, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0);
            (0..n)
                .map(|_| {
                    let w = rng.bi();
                    b0 = 0.99886 * b0 + w * 0.0555179;
                    b1 = 0.99332 * b1 + w * 0.0750759;
                    b2 = 0.96900 * b2 + w * 0.153_852;
                    b3 = 0.86650 * b3 + w * 0.3104856;
                    b4 = 0.55000 * b4 + w * 0.5329522;
                    b5 = -0.7616 * b5 - w * 0.0168980;
                    let out = b0 + b1 + b2 + b3 + b4 + b5 + b6 + w * 0.5362;
                    b6 = w * 0.115926;
                    out * 0.11
                })
                .collect()
        }
        NoiseColor::Brown => {
            // Leaky integration of white noise.
            let mut last = 0.0f32;
            (0..n)
                .map(|_| {
                    last = (last + 0.02 * rng.bi()) * 0.998;
                    (last * 8.0).clamp(-1.0, 1.0)
                })
                .collect()
        }
    }
}

/// Two-operator FM: carrier phase modulated by an operator at `freq * ratio`.
fn fm_signal(freq: &Value, ratio: f32, index: &Value, n: usize, sr: u32) -> Signal {
    let f = eval_value(freq, n, sr);
    let idx = eval_value(index, n, sr);
    let srf = sr as f32;
    let (mut cph, mut mph) = (0.0f32, 0.0f32);
    let mut out = Vec::with_capacity(n);
    for i in 0..n {
        let m = idx[i] * (TAU * mph).sin();
        out.push((TAU * cph + m).sin());
        let fi = f[i].max(0.0);
        cph += fi / srf;
        cph -= cph.floor();
        mph += (fi * ratio) / srf;
        mph -= mph.floor();
    }
    out
}

/// The per-seq instrument settings shared by every note.
struct SeqVoice<'a> {
    wave: SeqWave,
    duty: &'a Value,
    fm_ratio: f32,
    fm_index: f32,
    fm_strike: f32,
    pluck_decay: f32,
    // Guitar tone stages (the `pluck` voice).
    pluck_body: f32,
    pluck_pick: f32,
    pluck_tone: f32,
    // Piano tone knobs (engine-3 additive `piano` voice).
    piano_hammer: f32,
    piano_strike: f32,
    piano_inharm: f32,
    piano_detune: f32,
    piano_decay: f32,
    // Drum-kit voicing (the `kit` wave).
    kit: KitStyle,
    // Bass tone knobs (the `bass` wave).
    bass_cutoff: f32,
    bass_env: f32,
    bass_env_vel: f32,
    bass_decay: f32,
    bass_click: f32,
    bass_body: f32,
    bass_sub: f32,
    bass_sub_ratio: f32,
    bass_drive: f32,
    bass_body_decay: f32,
    // Read only by the SoundFont sampler path (feature = "sampler").
    #[cfg_attr(not(feature = "sampler"), allow(dead_code))]
    sf2: &'a str,
    #[cfg_attr(not(feature = "sampler"), allow(dead_code))]
    sf2_preset: u32,
    #[cfg_attr(not(feature = "sampler"), allow(dead_code))]
    sf2_bank: u32,
    swing: f32,
    humanize: f32,
    env: &'a Adsr,
    /// The document's engine revision — gates byte-changing voice upgrades
    /// (e.g. the engine-3 inharmonic `piano`).
    engine: u32,
}

/// Groove placement for one note: its start sample (swing + humanize timing)
/// and its humanized gain.
fn groove_note(note: &SeqNote, voice: &SeqVoice, step_dur: f32) -> (usize, f32) {
    // Swing delays every off-beat (odd) step by a fraction of a step;
    // humanize adds a deterministic per-note timing push/pull and velocity
    // wobble so repeats stop sounding machine-perfect.
    let swing_delay = if note.step % 2 == 1 {
        voice.swing * 0.5 * step_dur
    } else {
        0.0
    };
    let (human_delay, gain) = if voice.humanize > 0.0 {
        // Seed from the note's identity so the jitter is stable per note.
        let mut hr = Rng::new((note.step as u64) << 32 ^ (note.len as u64) << 8 ^ 0x6A09_E667);
        (
            voice.humanize * 0.12 * step_dur * hr.bi(),
            note.gain * (1.0 + voice.humanize * 0.15 * hr.bi()),
        )
    } else {
        (0.0, note.gain)
    };
    let start = (note.step as f32 * step_dur + swing_delay + human_delay).max(0.0) as usize;
    (start, gain.clamp(0.0, 1.0))
}

/// Render a note sequence: each note is an instrument voice with its own
/// pitch, length, and the shared per-note ADSR, summed into the output
/// (polyphonic).
fn render_seq(
    bpm: f32,
    steps_per_beat: u32,
    voice: &SeqVoice,
    notes: &[SeqNote],
    n: usize,
    sr: u32,
    rng: &mut Rng,
) -> Signal {
    let srf = sr as f32;
    let step_dur = srf * 60.0 / bpm / steps_per_beat.max(1) as f32; // samples per step
    // The sampler plays all notes through one shared synthesizer (voices
    // interact via polyphony), so it renders the sequence as a whole.
    #[cfg(feature = "sampler")]
    if voice.wave == SeqWave::Sampler {
        return sampler_seq(voice, notes, step_dur, n, sr);
    }
    #[cfg(not(feature = "sampler"))]
    if voice.wave == SeqWave::Sampler {
        return vec![0.0f32; n];
    }
    let mut out = vec![0.0f32; n];
    for note in notes {
        let (start, gain) = groove_note(note, voice, step_dur);
        if start >= n {
            continue;
        }
        // Bound the note length by the render window BEFORE allocating: a huge
        // note.len (or tiny bpm) must not size buffers beyond what's audible.
        // (f32→usize saturates, so even an inf product stays capped by n.)
        let len = ((note.len as f32 * step_dur).min(n as f32) as usize).max(1);
        let avail = (n - start).min(len);
        let envb = adsr(voice.env, len, sr);
        let f = eval_value(&note.pitch, len, sr);
        let d = eval_value(voice.duty, len, sr);
        let sig = seq_note_signal(voice, note, &f[..avail], &d[..avail], sr, rng);
        for (i, s) in sig.into_iter().enumerate() {
            out[start + i] += s * envb[i] * gain;
        }
    }
    out
}

/// Render a `Node::Seq` to a mono buffer with the given RNG — the exact seq
/// synthesis, shared by the offline renderer and the streaming renderer (which
/// pre-renders the seq with a structurally-seeded RNG) so a streamed seq is
/// byte-identical. Silence for a non-Seq node.
pub(crate) fn seq_to_signal(node: &Node, n: usize, sr: u32, rng: &mut Rng, engine: u32) -> Signal {
    if let Node::Seq {
        bpm,
        steps_per_beat,
        wave,
        duty,
        fm_ratio,
        fm_index,
        fm_strike,
        pluck_decay,
        pluck_body,
        pluck_pick,
        pluck_tone,
        piano_hammer,
        piano_strike,
        piano_inharm,
        piano_detune,
        piano_decay,
        kit,
        bass_cutoff,
        bass_env,
        bass_env_vel,
        bass_decay,
        bass_click,
        bass_body,
        bass_sub,
        bass_sub_ratio,
        bass_drive,
        bass_body_decay,
        sf2,
        sf2_preset,
        sf2_bank,
        swing,
        humanize,
        env,
        notes,
    } = node
    {
        let voice = SeqVoice {
            wave: *wave,
            duty,
            fm_ratio: *fm_ratio,
            fm_index: *fm_index,
            fm_strike: *fm_strike,
            pluck_decay: *pluck_decay,
            pluck_body: *pluck_body,
            pluck_pick: *pluck_pick,
            pluck_tone: *pluck_tone,
            piano_hammer: *piano_hammer,
            piano_strike: *piano_strike,
            piano_inharm: *piano_inharm,
            piano_detune: *piano_detune,
            piano_decay: *piano_decay,
            kit: *kit,
            bass_cutoff: *bass_cutoff,
            bass_env: *bass_env,
            bass_env_vel: *bass_env_vel,
            bass_decay: *bass_decay,
            bass_click: *bass_click,
            bass_body: *bass_body,
            bass_sub: *bass_sub,
            bass_sub_ratio: *bass_sub_ratio,
            bass_drive: *bass_drive,
            bass_body_decay: *bass_body_decay,
            sf2,
            sf2_preset: *sf2_preset,
            sf2_bank: *sf2_bank,
            swing: *swing,
            humanize: *humanize,
            env,
            engine,
        };
        render_seq(*bpm, *steps_per_beat, &voice, notes, n, sr, rng)
    } else {
        vec![0.0; n]
    }
}

/// Render one note of a seq instrument: `f`/`d` are the per-sample pitch and
/// duty buffers (already truncated to the audible window). Each instrument
/// owns its per-note state; instruments that consume the PRNG (noise, pluck,
/// piano's thump, the kit) draw in sample order, keeping renders byte-exact.
fn seq_note_signal(
    voice: &SeqVoice,
    note: &SeqNote,
    f: &[f32],
    d: &[f32],
    sr: u32,
    rng: &mut Rng,
) -> Signal {
    let srf = sr as f32;
    let n = f.len();
    let mut out = Vec::with_capacity(n);
    match voice.wave {
        SeqWave::Square => {
            let mut phase = 0.0f32;
            for i in 0..n {
                let dt = f[i].max(0.0) / srf;
                let duty = d[i].clamp(0.01, 0.99);
                let mut v = if phase < duty { 1.0 } else { -1.0 };
                v += poly_blep(phase, dt);
                v -= poly_blep((phase - duty + 1.0).fract(), dt);
                out.push(v);
                phase += dt;
                phase -= phase.floor();
            }
        }
        SeqWave::Triangle => {
            let (mut phase, mut tri) = (0.0f32, 0.0f32);
            for &fi in f {
                let dt = fi.max(0.0) / srf;
                let mut sq = if phase < 0.5 { 1.0 } else { -1.0 };
                sq += poly_blep(phase, dt);
                sq -= poly_blep((phase + 0.5).fract(), dt);
                tri = tri * 0.9995 + 4.0 * dt * sq;
                out.push(tri);
                phase += dt;
                phase -= phase.floor();
            }
        }
        SeqWave::Sawtooth => {
            let mut phase = 0.0f32;
            for &fi in f {
                let dt = fi.max(0.0) / srf;
                out.push((2.0 * phase - 1.0) - poly_blep(phase, dt));
                phase += dt;
                phase -= phase.floor();
            }
        }
        SeqWave::Sine => {
            let mut phase = 0.0f32;
            for &fi in f {
                out.push(osc(Shape::Sine, phase));
                phase += fi.max(0.0) / srf;
                phase -= phase.floor();
            }
        }
        SeqWave::Noise => out.extend((0..n).map(|_| rng.bi())),
        SeqWave::Fm => {
            let (mut cph, mut mph) = (0.0f32, 0.0f32);
            for (i, &fi) in f.iter().enumerate() {
                let dt = fi.max(0.0) / srf;
                // Hammer strike: the modulation index (brightness) decays
                // from the attack; louder notes strike brighter.
                let t = i as f32 / srf;
                let idx = voice.fm_index
                    * (0.4 + 0.6 * note.gain)
                    * (-t / voice.fm_strike.max(1e-3)).exp();
                let m = idx * (TAU * mph).sin();
                out.push((TAU * cph + m).sin());
                cph += dt;
                cph -= cph.floor();
                mph += dt * voice.fm_ratio;
                mph -= mph.floor();
            }
        }
        SeqWave::Pluck => {
            // Karplus-Strong: a noise burst in a delay line tuned to the note's
            // onset pitch. Three RNG-free stages wrap the loop — a tunable
            // damping/brightness filter (`pluck_tone`), a fixed guitar-body mode
            // bank (`pluck_body`), and a pick-attack click (`pluck_pick`) — each
            // an identity op at its default, so the plain pluck is byte-identical
            // and the `period` noise draws are unchanged.
            let period = ((srf / f[0].clamp(20.0, srf / 2.0)).round() as usize).max(2);
            let mut string: Vec<f32> = (0..period).map(|_| rng.bi()).collect();
            let mut spos = 0usize;
            let bright = voice.pluck_tone.max(0.0);
            let damp = (-voice.pluck_tone).max(0.0);
            // Fixed guitar-body resonators: Helmholtz air, top plate, back.
            let body_r = (-6.907_755 / (0.25 * srf)).exp();
            let body_a2 = -body_r * body_r;
            let body: [(f32, f32); 3] =
                [(100.0, 1.0), (215.0, 0.8), (400.0, 0.5)].map(|(fr, g)| {
                    let w0 = TAU * fr / srf;
                    (2.0 * body_r * w0.cos(), g * w0.sin()) // (a1, b0)
                });
            let (mut by1, mut by2) = ([0.0f32; 3], [0.0f32; 3]);
            let (mut lp, mut hp_in, mut hp_out) = (0.0f32, 0.0f32, 0.0f32);
            for i in 0..n {
                let t = i as f32 / srf;
                let y = string[spos];
                let next = string[(spos + 1) % string.len()];
                // Pick click: the highpassed leading edge of the excitation.
                let pick = if t < 0.008 {
                    let hp = 0.9 * (hp_out + y - hp_in);
                    hp_in = y;
                    hp_out = hp;
                    voice.pluck_pick * hp * (1.0 - t / 0.008)
                } else {
                    0.0
                };
                // Body resonance driven by the string output.
                let mut body_sum = 0.0f32;
                for k in 0..3 {
                    let (a1, b0) = body[k];
                    let yr = b0 * y + a1 * by1[k] + body_a2 * by2[k];
                    by2[k] = by1[k];
                    by1[k] = yr;
                    body_sum += yr;
                }
                let out_sample =
                    (1.0 - 0.3 * voice.pluck_body) * y + voice.pluck_body * 0.6 * body_sum + pick;
                out.push(out_sample);
                // Loop filter: brightness blend then a darkening one-pole.
                let avg = (0.5 + 0.5 * bright) * y + (0.5 - 0.5 * bright) * next;
                lp += damp * (avg - lp);
                let filt = (1.0 - damp) * avg + damp * lp;
                string[spos] = voice.pluck_decay * filt;
                spos = (spos + 1) % string.len();
            }
        }
        SeqWave::Piano if voice.engine >= 3 => {
            // Inharmonic additive grand (engine 3). A real piano string is stiff,
            // so its partials stretch sharp: fₖ = k·f₀·√(1 + B·k²). Each partial
            // owns its decay (highs die first — the bright attack mellowing to a
            // warm sustain), a hammer-strike spectrum (a 1/k tilt with a notch at
            // the ~1/8 strike point, opened by velocity), over a detuned unison
            // pair whose slow beating is the shimmer. Bass rings for seconds,
            // treble dies fast.
            struct Partial {
                step: f32, // inharmonic frequency ratio to the fundamental
                amp: f32,  // hammer-spectrum amplitude
                env: f32,  // current decay level
                dmul: f32, // per-sample decay multiplier
                phase: [f32; 2],
            }
            // Five tone knobs (defaults reproduce the concert grand bit-for-bit).
            let f0 = f[0].max(20.0);
            let b_inharm = (7.0e-5 * voice.piano_inharm * (f0 / 55.0))
                .clamp(5.0e-5 * voice.piano_inharm, 1.2e-3 * voice.piano_inharm);
            let base_decay = (10.0 * voice.piano_decay / (1.0 + f0 / 110.0)).clamp(0.45, 9.0);
            let strike = voice.piano_strike.clamp(0.01, 0.5); // hammer position along the string
            let bright = 0.45 + 0.55 * note.gain; // velocity opens the high partials
            let hammer = voice.piano_hammer.max(1e-3); // hardness: flattens the tilt
            let detune = 1.0 + (1.000_6_f32 - 1.0) * voice.piano_detune; // unison spread
            let string_det = [1.0 / detune, detune];

            let mut partials: Vec<Partial> = Vec::new();
            let mut k = 1usize;
            while k <= 18 {
                let kf = k as f32;
                let ratio = kf * (1.0 + b_inharm * kf * kf).sqrt();
                if ratio * f0 > 0.45 * srf {
                    break; // keep every partial below Nyquist
                }
                let notch = (std::f32::consts::PI * kf * strike).sin().abs();
                let amp = notch / kf * bright.powf((kf - 1.0) * 0.18 / hammer);
                let decay = (base_decay / (1.0 + 0.55 * (kf - 1.0))).max(0.05);
                partials.push(Partial {
                    step: ratio,
                    amp,
                    env: 1.0,
                    dmul: (-1.0 / (srf * decay)).exp(),
                    // Spread start phases (golden ratio) so the onset isn't a
                    // hard in-phase transient — deterministic, no RNG draw.
                    phase: [(kf * 0.618_034).fract(), (kf * 0.381_966).fract()],
                });
                k += 1;
            }
            // Target a per-note peak near 0.5 (as the FM model had): two strings
            // over the summed partial amplitude.
            let norm = 0.5 / (2.0 * partials.iter().map(|p| p.amp).sum::<f32>().max(1e-6));

            for (i, &fi) in f.iter().enumerate() {
                let dt = fi.max(0.0) / srf;
                let t = i as f32 / srf;
                let mut s = 0.0;
                for p in partials.iter_mut() {
                    let inc = dt * p.step;
                    let a = p.amp * p.env;
                    for (ph, &det) in p.phase.iter_mut().zip(string_det.iter()) {
                        s += a * (TAU * *ph).sin();
                        *ph += inc * det;
                        *ph -= ph.floor();
                    }
                    p.env *= p.dmul;
                }
                // Felt-hammer thump: a few ms of soft noise on the attack.
                let thump = if t < 0.006 {
                    rng.bi() * 0.3 * (1.0 - t / 0.006)
                } else {
                    0.0
                };
                out.push(s * norm + thump);
            }
        }
        SeqWave::Piano => {
            // Two strings detuned ±1.6 cents beat slowly against each other —
            // the chorusing shimmer of a real unison pair. Natural decay time
            // falls with pitch: bass strings ring for seconds, treble dies
            // in under one.
            let decay = (8.0 / (1.0 + f[0].max(20.0) / 110.0)).clamp(0.25, 6.0);
            let detune = 1.000_92; // 2^(1.6/1200)
            let (mut cph, mut mph) = (0.0f32, 0.0f32);
            let (mut cph2, mut mph2) = (0.0f32, 0.0f32);
            for (i, &fi) in f.iter().enumerate() {
                let dt = fi.max(0.0) / srf;
                let t = i as f32 / srf;
                // Hammer-strike brightness: louder keys strike brighter and
                // the shimmer fades within ~80 ms.
                let idx = (1.2 + 2.3 * note.gain) * (-t / 0.08).exp();
                let a = (TAU * cph + idx * (TAU * mph).sin()).sin();
                let b = (TAU * cph2 + idx * (TAU * mph2).sin()).sin();
                cph += dt / detune;
                cph -= cph.floor();
                mph += dt / detune;
                mph -= mph.floor();
                cph2 += dt * detune;
                cph2 -= cph2.floor();
                mph2 += dt * detune;
                mph2 -= mph2.floor();
                // Felt-hammer thump: 4 ms of soft noise on the attack.
                let thump = if t < 0.004 {
                    rng.bi() * 0.25 * (1.0 - t / 0.004)
                } else {
                    0.0
                };
                out.push((0.5 * (a + b) + thump) * (-t / decay).exp());
            }
        }
        SeqWave::Epiano => {
            // Rhodes-style: a soft FM body (1:1) under a metal tine (14:1)
            // that pings on the attack. Velocity opens the tine.
            let decay = (5.0 / (1.0 + f[0].max(20.0) / 250.0)).clamp(0.3, 4.0);
            let (mut cph, mut mph, mut tph) = (0.0f32, 0.0f32, 0.0f32);
            for (i, &fi) in f.iter().enumerate() {
                let dt = fi.max(0.0) / srf;
                let t = i as f32 / srf;
                let body_idx = (0.5 + 1.0 * note.gain) * (-t / 0.5).exp();
                let tine_idx = (0.8 + 1.4 * note.gain) * (-t / 0.035).exp();
                let body = (TAU * cph + body_idx * (TAU * mph).sin()).sin();
                let tine = (TAU * cph + tine_idx * (TAU * tph).sin()).sin();
                cph += dt;
                cph -= cph.floor();
                mph += dt;
                mph -= mph.floor();
                tph += dt * 14.0;
                tph -= tph.floor();
                out.push((0.75 * body + 0.25 * tine) * (-t / decay).exp());
            }
        }
        SeqWave::Organ => {
            // Tonewheel drawbars over half the fundamental (so the 16′ bar is
            // an integer partial and every phase wraps cleanly): 16′ 8′ 4′
            // 2⅔′ 2′, plus the classic percussion ping on the attack.
            const BARS: [(f32, f32); 5] = [
                (1.0, 0.45),
                (2.0, 1.0),
                (4.0, 0.45),
                (6.0, 0.3),
                (8.0, 0.22),
            ];
            let norm = 1.0 / BARS.iter().map(|(_, g)| g).sum::<f32>();
            let mut phase = 0.0f32; // at f/2
            for (i, &fi) in f.iter().enumerate() {
                let t = i as f32 / srf;
                let mut s = 0.0;
                for (k, g) in BARS {
                    s += g * (TAU * phase * k).sin();
                }
                // Percussion: a 3rd-harmonic ping that fades in 200 ms.
                s += 0.5 * (-t / 0.2).exp() * (TAU * phase * 6.0).sin();
                out.push(s * norm);
                phase += fi.max(0.0) / 2.0 / srf;
                // Wrap on the full drawbar cycle to keep precision.
                phase -= phase.floor();
            }
        }
        SeqWave::Strings => {
            // Ensemble: three saws detuned ±8 cents, phase-spread, swelling
            // in like a bow stroke, mellowed by a one-pole lowpass.
            let detunes = [0.995_39f32, 1.0, 1.004_63]; // ∓8 cents
            let mut phases = [0.0f32, 0.33, 0.67];
            let lp_a = 1.0 - (-TAU * 3_000.0 / srf).exp();
            let mut lp = 0.0f32;
            for (i, &fi) in f.iter().enumerate() {
                let t = i as f32 / srf;
                let mut s = 0.0;
                for (p, det) in phases.iter_mut().zip(detunes) {
                    let dt = fi.max(0.0) * det / srf;
                    s += (2.0 * *p - 1.0) - poly_blep(*p, dt);
                    *p += dt;
                    *p -= p.floor();
                }
                lp += lp_a * (s / 3.0 - lp);
                let swell = 1.0 - (-t / 0.12).exp();
                out.push(lp * swell);
            }
        }
        SeqWave::Bass => {
            // A saw through a velocity-swept one-pole lowpass over a sine sub.
            // Every constant is a `bass_*` knob; the defaults reproduce the
            // original fingered bass bit-for-bit (and draw no RNG, so it streams
            // byte-identically). `bass_click` adds a deterministic pick tick,
            // `bass_drive` a tanh grit, `bass_sub_ratio` an octave-down sub.
            const BASS_CLICK_TAU: f32 = 0.008;
            let decay = voice.bass_decay.max(1e-3);
            let body_decay = voice.bass_body_decay.max(1e-3);
            let drive = voice.bass_drive.clamp(0.0, 1.0);
            let mut phase = 0.0f32;
            let mut sub_phase = 0.0f32;
            let mut lp = 0.0f32;
            for (i, &fi) in f.iter().enumerate() {
                let dt = fi.max(0.0) / srf;
                let t = i as f32 / srf;
                let saw = (2.0 * phase - 1.0) - poly_blep(phase, dt);
                let cutoff = voice.bass_cutoff
                    + (voice.bass_env + voice.bass_env_vel * note.gain) * (-t / decay).exp()
                    + voice.bass_click * (-t / BASS_CLICK_TAU).exp();
                let a = 1.0 - (-TAU * cutoff / srf).exp();
                lp += a * (saw - lp);
                let body = lp + drive * ((lp * (1.0 + 2.0 * drive)).tanh() - lp);
                let sub = (TAU * sub_phase).sin();
                out.push((voice.bass_body * body + voice.bass_sub * sub) * (-t / body_decay).exp());
                phase += dt;
                phase -= phase.floor();
                sub_phase += dt * voice.bass_sub_ratio;
                sub_phase -= sub_phase.floor();
            }
        }
        SeqWave::Kit => out = kit_drum(f, note, sr, rng, voice.kit),
        // Handled wholesale in sampler_seq (shared synthesizer, polyphony).
        SeqWave::Sampler => unreachable!("sampler renders via sampler_seq"),
        SeqWave::Cowbell => {
            for (i, &fi) in f.iter().enumerate() {
                let t = i as f32 / srf;
                out.push(cowbell_sample(fi.max(20.0), t));
            }
        }
    }
    out
}

/// One sample of cowbell at fundamental `f`: two clashing partials (the
/// classic ~1.56 ratio of an 808 cowbell), saturated square-ish, with a fast
/// knock decay.
fn cowbell_sample(f: f32, t: f32) -> f32 {
    let a = (2.5 * (TAU * f * t).sin()).tanh();
    let b = (2.5 * (TAU * f * 1.565 * t).sin()).tanh();
    0.5 * (a + b) * (-t / 0.09).exp()
}

/// Render a whole sampler seq through rustysynth: real recorded instruments
/// from a SoundFont. All notes share one synthesizer so polyphony, voice
/// stealing, and per-preset envelopes behave like a real MIDI instrument.
/// Output is the stereo render downmixed to the graph's mono bus (doc-level
/// `stereo` adds width back at the output stage).
#[cfg(feature = "sampler")]
fn sampler_seq(voice: &SeqVoice, notes: &[SeqNote], step_dur: f32, n: usize, sr: u32) -> Signal {
    match sampler_seq_stereo(voice, notes, step_dur, n, sr) {
        Some((l, r)) => l.iter().zip(r).map(|(a, b)| 0.5 * (a + b)).collect(),
        None => vec![0.0; n],
    }
}

/// The sampler's native stereo render (used directly by mixer tracks).
#[cfg(feature = "sampler")]
fn sampler_seq_stereo(
    voice: &SeqVoice,
    notes: &[SeqNote],
    step_dur: f32,
    n: usize,
    sr: u32,
) -> Option<(Signal, Signal)> {
    use rustysynth::{Synthesizer, SynthesizerSettings};

    let font = match load_soundfont(voice.sf2) {
        Ok(f) => f,
        Err(e) => {
            tracing::warn!("sampler: cannot load '{}': {e}", voice.sf2);
            return None;
        }
    };
    let mut settings = SynthesizerSettings::new(sr as i32);
    // Our graph supplies reverb/chorus as explicit processors; the synth's
    // built-ins stay off so renders are lean and deterministic.
    settings.enable_reverb_and_chorus = false;
    let mut synth = match Synthesizer::new(&font, &settings) {
        Ok(s) => s,
        Err(e) => {
            tracing::warn!("sampler: synthesizer init failed: {e:?}");
            return None;
        }
    };
    // Channel 9 is percussion by MIDI convention; bank 128 selects it.
    let ch = if voice.sf2_bank == 128 { 9 } else { 0 };
    synth.process_midi_message(ch, 0xC0, voice.sf2_preset.min(127) as i32, 0);

    // Schedule note on/offs on the sample timeline (groove applied).
    let mut events: Vec<(usize, bool, i32, i32)> = Vec::with_capacity(notes.len() * 2);
    for note in notes {
        let (start, gain) = groove_note(note, voice, step_dur);
        if start >= n {
            continue;
        }
        let len = ((note.len as f32 * step_dur).min(n as f32) as usize).max(1);
        let hz = eval_value(&note.pitch, 1, sr)[0].max(8.0);
        let key = (69.0 + 12.0 * (hz / 440.0).log2()).round() as i32;
        let vel = ((gain * 127.0) as i32).clamp(1, 127);
        events.push((start, true, key.clamp(0, 127), vel));
        events.push(((start + len).min(n), false, key.clamp(0, 127), 0));
    }
    // Offs before ons at the same instant, so retriggers restart the voice.
    events.sort_by_key(|&(at, is_on, ..)| (at, is_on));

    let (mut left, mut right) = (vec![0.0f32; n], vec![0.0f32; n]);
    let mut pos = 0usize;
    for (at, is_on, key, vel) in events {
        if at > pos {
            let (lh, rh) = (&mut left[pos..at], &mut right[pos..at]);
            synth.render(lh, rh);
            pos = at;
        }
        if is_on {
            synth.note_on(ch, key, vel);
        } else {
            synth.note_off(ch, key);
        }
    }
    if pos < n {
        synth.render(&mut left[pos..], &mut right[pos..]);
    }
    Some((left, right))
}

/// SoundFonts are large; load each file once per process and share it.
#[cfg(feature = "sampler")]
fn load_soundfont(path: &str) -> anyhow::Result<std::sync::Arc<rustysynth::SoundFont>> {
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex, OnceLock};
    static CACHE: OnceLock<Mutex<HashMap<String, Arc<rustysynth::SoundFont>>>> = OnceLock::new();
    let cache = CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    if let Some(f) = cache.lock().unwrap_or_else(|e| e.into_inner()).get(path) {
        return Ok(f.clone());
    }
    let mut file = std::fs::File::open(path)?;
    let font = Arc::new(
        rustysynth::SoundFont::new(&mut file).map_err(|e| anyhow::anyhow!("parse: {e:?}"))?,
    );
    cache
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(path.to_string(), font.clone());
    Ok(font)
}

/// One General-MIDI-mapped drum hit: the note's onset pitch picks the voice.
/// Synthesize one drum hit for the selected kit style. `Classic` is the original
/// kit, byte-frozen; the other styles are alternate synthesized voicings.
fn kit_drum(f: &[f32], note: &SeqNote, sr: u32, rng: &mut Rng, style: KitStyle) -> Signal {
    match style {
        KitStyle::Classic => kit_drum_classic(f, note, sr, rng),
        KitStyle::Acoustic => kit_drum_acoustic(f, note, sr, rng),
        KitStyle::Electronic => kit_drum_electronic(f, note, sr, rng),
        KitStyle::Eight08 => kit_drum_808(f, note, sr, rng),
    }
}

fn kit_drum_classic(f: &[f32], _note: &SeqNote, sr: u32, rng: &mut Rng) -> Signal {
    let srf = sr as f32;
    let n = f.len();
    // Recover the MIDI number from the onset frequency (pitch is wire-encoded
    // as Hz; "midi:36" round-trips exactly).
    let midi = (69.0 + 12.0 * (f[0].max(8.0) / 440.0).log2()).round() as i32;
    let mut out = Vec::with_capacity(n);

    // One-pole highpass state for cymbal/snare noise.
    let (mut lp, hp_a) = (0.0f32, 1.0 - (-TAU * 5_500.0 / srf).exp());
    let hp = |x: f32, lp: &mut f32| {
        *lp += hp_a * (x - *lp);
        x - *lp
    };
    let mut phase = 0.0f32;

    for i in 0..n {
        let t = i as f32 / srf;
        let s = match midi {
            // Kick: a fast downward pitch thump plus a 2 ms beater click.
            35 | 36 => {
                let fk = 45.0 + 105.0 * (-t / 0.04).exp();
                phase += fk / srf;
                phase -= phase.floor();
                let click = if t < 0.002 { rng.bi() * 0.4 } else { 0.0 };
                (TAU * phase).sin() * (-t / 0.13).exp() + click
            }
            // Snare / rimshot / clap: tone crack + noise body.
            38 | 40 => {
                let tone = (TAU * 190.0 * t).sin() * 0.4 * (-t / 0.06).exp();
                tone + rng.bi() * 0.8 * (-t / 0.11).exp()
            }
            37 => (TAU * 800.0 * t).sin() * 0.3 * (-t / 0.03).exp() + rng.bi() * (-t / 0.025).exp(),
            39 => rng.bi() * (-t / 0.09).exp(),
            // Hats: highpassed noise, closed dies fast, open rings.
            42 | 44 => hp(rng.bi(), &mut lp) * (-t / 0.035).exp(),
            46 => hp(rng.bi(), &mut lp) * (-t / 0.22).exp(),
            // Toms: pitched thumps falling with the GM map.
            41 | 43 | 45 | 47 | 48 | 50 => {
                let base = 80.0 + 24.0 * (midi - 41) as f32;
                let ft = base * (1.0 - 0.15 * (t / 0.2).min(1.0));
                phase += ft / srf;
                phase -= phase.floor();
                (TAU * phase).sin() * (-t / 0.18).exp() + rng.bi() * 0.1 * (-t / 0.03).exp()
            }
            // Cowbell (more cowbell).
            56 => cowbell_sample(540.0, t),
            // Crash / ride.
            49 | 55 | 57 => hp(rng.bi(), &mut lp) * (-t / 0.7).exp(),
            51 | 53 | 59 => {
                hp(rng.bi(), &mut lp) * 0.5 * (-t / 0.45).exp()
                    + (TAU * 5_200.0 * t).sin() * 0.25 * (-t / 0.25).exp()
            }
            // Anything unmapped: a generic percussive hit.
            _ => rng.bi() * (-t / 0.08).exp(),
        };
        out.push(s);
    }
    out
}

// The alternate kit styles keep the classic GM note map and the per-sample /
// in-order-`rng.bi()` discipline so they stream byte-identically.

/// Deeper, roomier acoustic kit: pitch-dropping kick with a beater knock, a
/// tuned-head snare with crack and buzz, ringing toms, shimmering cymbals.
fn kit_drum_acoustic(f: &[f32], _note: &SeqNote, sr: u32, rng: &mut Rng) -> Signal {
    let srf = sr as f32;
    let n = f.len();
    let midi = (69.0 + 12.0 * (f[0].max(8.0) / 440.0).log2()).round() as i32;
    let a = |fc: f32| 1.0 - (-TAU * fc / srf).exp();
    let (a3000, a3500, a4000, a2500, a400, a900) = (
        a(3000.0),
        a(3500.0),
        a(4000.0),
        a(2500.0),
        a(400.0),
        a(900.0),
    );
    let (a11000, a8000, a6500, a2000, a12000, a7000) = (
        a(11000.0),
        a(8000.0),
        a(6500.0),
        a(2000.0),
        a(12000.0),
        a(7000.0),
    );
    let (mut lpa, mut lpb, mut hpa) = (0.0f32, 0.0f32, 0.0f32);
    let (mut phase, mut phase2) = (0.0f32, 0.0f32);
    let mut out = Vec::with_capacity(n);
    for i in 0..n {
        let t = i as f32 / srf;
        let s = match midi {
            35 | 36 => {
                let fk = 48.0 + 140.0 * (-t / 0.028).exp();
                phase += fk / srf;
                phase -= phase.floor();
                let body = (TAU * phase).sin() * (-t / 0.16).exp();
                let click = if t < 0.018 {
                    let w = rng.bi();
                    lpa += a3000 * (w - lpa);
                    let tick = (TAU * 2600.0 * t).sin() * (-t / 0.004).exp();
                    (w - lpa) * 0.50 * (-t / 0.007).exp() + 0.28 * tick
                } else {
                    0.0
                };
                0.90 * body + click
            }
            38 | 40 => {
                let w = rng.bi();
                let m1 = (TAU * 185.0 * t).sin() * 0.48 * (-t / 0.10).exp();
                let m2 = (TAU * 330.0 * t).sin() * 0.26 * (-t / 0.07).exp();
                lpa += a3500 * (w - lpa);
                let crack = (w - lpa) * 0.55 * (-t / 0.035).exp();
                lpb += a2500 * (w - lpb);
                hpa += a400 * (lpb - hpa);
                let buzz = (lpb - hpa) * 0.40 * (-t / 0.13).exp();
                m1 + m2 + crack + buzz
            }
            37 => {
                let w = rng.bi();
                lpa += a4000 * (w - lpa);
                let snap = (w - lpa) * 0.50 * (-t / 0.012).exp();
                let ring = (TAU * 1700.0 * t).sin() * 0.35 * (-t / 0.008).exp();
                let knock = (TAU * 420.0 * t).sin() * 0.30 * (-t / 0.03).exp();
                snap + ring + knock
            }
            39 => {
                let w = rng.bi();
                lpa += a2500 * (w - lpa);
                hpa += a900 * (lpa - hpa);
                let band = lpa - hpa;
                let burst = |d: f32| {
                    if t >= d {
                        (-(t - d) / 0.009).exp()
                    } else {
                        0.0
                    }
                };
                let bursts = (burst(0.0) + burst(0.009) + burst(0.018) + burst(0.027)).min(1.0);
                let tail = 0.35 * (-t / 0.10).exp();
                band * (0.90 * bursts + tail)
            }
            42 | 44 => {
                let w = rng.bi();
                lpa += a11000 * (w - lpa);
                hpa += a8000 * (lpa - hpa);
                let shimmer = lpa - hpa;
                lpb += a6500 * (w - lpb);
                (0.60 * (w - lpb) + 0.55 * shimmer) * (-t / 0.032).exp()
            }
            46 => {
                let w = rng.bi();
                lpa += a11000 * (w - lpa);
                hpa += a8000 * (lpa - hpa);
                let shimmer = lpa - hpa;
                lpb += a6500 * (w - lpb);
                let env = 0.85 * (-t / 0.32).exp() + 0.15 * (-t / 0.08).exp();
                (0.55 * (w - lpb) + 0.60 * shimmer) * env
            }
            41 | 43 | 45 | 47 | 48 | 50 => {
                let base = 80.0 + 24.0 * (midi - 41) as f32;
                let ft = base * (1.0 - 0.12 * (t / 0.25).min(1.0));
                phase += ft / srf;
                phase -= phase.floor();
                phase2 += 1.59 * ft / srf;
                phase2 -= phase2.floor();
                let fund = (TAU * phase).sin() * (-t / 0.35).exp();
                let mode = (TAU * phase2).sin() * 0.30 * (-t / 0.14).exp();
                let w = rng.bi();
                lpa += a2000 * (w - lpa);
                let stick = (w - lpa) * 0.18 * (-t / 0.008).exp();
                0.85 * fund + mode + stick
            }
            56 => cowbell_sample(540.0, t),
            49 | 55 | 57 => {
                let w = rng.bi();
                lpa += a2500 * (w - lpa);
                let wash = (w - lpa) * 0.60 * (-t / 0.90).exp();
                lpb += a12000 * (w - lpb);
                hpa += a7000 * (lpb - hpa);
                let lfo = 0.6 + 0.4 * (TAU * 6.0 * t).sin();
                let shine = (lpb - hpa) * lfo * 0.50 * (-t / 0.70).exp();
                let clash = ((TAU * 3300.0 * t).sin()
                    + (TAU * 5240.0 * t).sin()
                    + (TAU * 8130.0 * t).sin())
                    * 0.06
                    * (-t / 0.22).exp();
                wash + shine + clash
            }
            51 | 53 | 59 => {
                let w = rng.bi();
                lpa += a3000 * (w - lpa);
                let wash = (w - lpa) * 0.45 * (-t / 0.55).exp();
                lpb += a12000 * (w - lpb);
                hpa += a8000 * (lpb - hpa);
                let shine = (lpb - hpa) * 0.40 * (-t / 0.40).exp();
                let ping = ((TAU * 2100.0 * t).sin() * 0.5
                    + (TAU * 3170.0 * t).sin() * 0.3
                    + (TAU * 4200.0 * t).sin() * 0.2)
                    * (-t / 0.30).exp();
                0.50 * ping + wash + shine
            }
            _ => {
                let w = rng.bi();
                lpa += a4000 * (w - lpa);
                (0.5 * w + 0.5 * lpa) * (-t / 0.08).exp()
            }
        };
        out.push(s);
    }
    out
}

/// Clean synthesized electronic kit: driven synth kick, gated snare, zappy toms,
/// glassy super-bright hats and cymbals.
fn kit_drum_electronic(f: &[f32], _note: &SeqNote, sr: u32, rng: &mut Rng) -> Signal {
    let srf = sr as f32;
    let n = f.len();
    let midi = (69.0 + 12.0 * (f[0].max(8.0) / 440.0).log2()).round() as i32;
    let a5500 = 1.0 - (-TAU * 5500.0 / srf).exp();
    let a9000 = 1.0 - (-TAU * 9000.0 / srf).exp();
    let (mut lp, mut lp2, mut phase) = (0.0f32, 0.0f32, 0.0f32);
    let mut out = Vec::with_capacity(n);
    for i in 0..n {
        let t = i as f32 / srf;
        let s = match midi {
            35 | 36 => {
                let fk = 55.0 + 145.0 * (-t / 0.025).exp();
                phase += fk / srf;
                phase -= phase.floor();
                let body = (1.3 * (TAU * phase).sin()).tanh() * 0.85 * (-t / 0.11).exp();
                let click = if t < 0.003 { rng.bi() * 0.3 } else { 0.0 };
                body + click
            }
            38 | 40 => {
                let tone = ((TAU * 185.0 * t).sin() * 0.45 + (TAU * 330.0 * t).sin() * 0.22)
                    * (-t / 0.055).exp();
                let gate = if t < 0.13 {
                    1.0
                } else {
                    (-(t - 0.13) / 0.006).exp()
                };
                let w = rng.bi();
                lp += a5500 * (w - lp);
                tone + (w - lp) * 0.7 * (-t / 0.16).exp() * gate
            }
            37 => {
                (TAU * 1700.0 * t).sin() * 0.5 * (-t / 0.012).exp()
                    + (TAU * 420.0 * t).sin() * 0.3 * (-t / 0.02).exp()
                    + rng.bi() * 0.3 * (-t / 0.006).exp()
            }
            39 => {
                let ev = if t < 0.03 {
                    (-((t % 0.01) / 0.003)).exp()
                } else {
                    (-(t - 0.03) / 0.10).exp()
                };
                let w = rng.bi();
                lp += a5500 * (w - lp);
                (0.5 * (w - lp) + 0.5 * w) * ev * 0.9
            }
            42 | 44 => {
                let w = rng.bi();
                lp2 += a9000 * (w - lp2);
                (w - lp2) * 1.5 * (-t / 0.02).exp()
            }
            46 => {
                let w = rng.bi();
                lp2 += a9000 * (w - lp2);
                (w - lp2) * 1.4 * (-t / 0.18).exp()
                    + (TAU * 9000.0 * t).sin() * (TAU * 11500.0 * t).sin() * 0.1 * (-t / 0.14).exp()
            }
            41 | 43 | 45 | 47 | 48 | 50 => {
                let base = 90.0 + 26.0 * (midi - 41) as f32;
                let ft = base * (1.0 + 1.5 * (-t / 0.05).exp());
                phase += ft / srf;
                phase -= phase.floor();
                (1.2 * (TAU * phase).sin()).tanh() * (-t / 0.16).exp()
                    + rng.bi() * 0.08 * (-t / 0.02).exp()
            }
            56 => cowbell_sample(555.0, t),
            49 | 55 | 57 => {
                let w = rng.bi();
                lp2 += a9000 * (w - lp2);
                (w - lp2) * 1.4 * (-t / 0.6).exp()
                    + (TAU * 8000.0 * t).sin() * (TAU * 11000.0 * t).sin() * 0.12 * (-t / 0.5).exp()
            }
            51 | 53 | 59 => {
                let w = rng.bi();
                lp2 += a9000 * (w - lp2);
                (TAU * 5800.0 * t).sin() * 0.35 * (-t / 0.35).exp()
                    + (TAU * 8700.0 * t).sin() * 0.15 * (-t / 0.3).exp()
                    + (w - lp2) * 0.4 * (-t / 0.5).exp()
            }
            _ => {
                let w = rng.bi();
                lp2 += a9000 * (w - lp2);
                (w - lp2) * 1.6 * (-t / 0.06).exp()
            }
        };
        out.push(s);
    }
    out
}

/// The Roland TR-808 hi-hat/cymbal oscillator bank: six hard-square partials.
fn metal_808(t: f32) -> f32 {
    const FS: [f32; 6] = [205.3, 304.4, 369.6, 522.7, 540.0, 800.0];
    let mut s = 0.0;
    for &fr in &FS {
        s += (TAU * fr * t).sin().signum();
    }
    s / 6.0
}

/// 808-style kit: a long booming sub-sine kick, papery clap, snappy snare, tick
/// clave, ringy square cowbell, buzzy metallic hats/cymbals.
fn kit_drum_808(f: &[f32], _note: &SeqNote, sr: u32, rng: &mut Rng) -> Signal {
    let srf = sr as f32;
    let n = f.len();
    let midi = (69.0 + 12.0 * (f[0].max(8.0) / 440.0).log2()).round() as i32;
    let a6000 = 1.0 - (-TAU * 6000.0 / srf).exp();
    let clo_a = 1.0 - (-TAU * 2200.0 / srf).exp();
    let chi_a = 1.0 - (-TAU * 700.0 / srf).exp();
    let (mut hlp, mut clp, mut chp, mut phase) = (0.0f32, 0.0f32, 0.0f32, 0.0f32);
    let mut out = Vec::with_capacity(n);
    for i in 0..n {
        let t = i as f32 / srf;
        let s = match midi {
            35 | 36 => {
                let fk = 52.0 + 68.0 * (-t / 0.025).exp();
                phase += fk / srf;
                phase -= phase.floor();
                let body = (TAU * phase).sin() * (-t / 0.60).exp();
                let click = if t < 0.004 {
                    (rng.bi() * 0.5 + (TAU * 1600.0 * t).sin() * 0.5) * (-t / 0.0015).exp()
                } else {
                    0.0
                };
                body + click * 0.5
            }
            38 | 40 => {
                let tone =
                    ((TAU * 175.0 * t).sin() + (TAU * 330.0 * t).sin()) * 0.32 * (-t / 0.10).exp();
                let w = rng.bi();
                hlp += a6000 * (w - hlp);
                tone + (w - hlp) * 0.7 * (-t / 0.07).exp()
            }
            37 => {
                let tick = (TAU * 1700.0 * t).sin() * 0.7 * (-t / 0.006).exp();
                let snap = if t < 0.003 {
                    rng.bi() * 0.3 * (-t / 0.001).exp()
                } else {
                    0.0
                };
                tick + snap
            }
            39 => {
                let ph = (t % 0.010) / 0.010;
                let burst = if t < 0.030 { (-ph / 0.22).exp() } else { 0.0 };
                let env = burst + 0.55 * (-t / 0.12).exp();
                let w = rng.bi();
                clp += clo_a * (w - clp);
                chp += chi_a * (clp - chp);
                (clp - chp) * env * 1.1
            }
            42 | 44 => {
                let m = metal_808(t);
                hlp += a6000 * (m - hlp);
                (m - hlp) * 0.55 * (-t / 0.05).exp()
            }
            46 => {
                let m = metal_808(t);
                hlp += a6000 * (m - hlp);
                (m - hlp) * 0.55 * (-t / 0.35).exp()
            }
            41 | 43 | 45 | 47 | 48 | 50 => {
                let base = 90.0 + 26.0 * (midi - 41) as f32;
                let ft = base * (1.0 + 0.6 * (-t / 0.02).exp());
                phase += ft / srf;
                phase -= phase.floor();
                let dec = 0.32 - 0.025 * (midi - 41) as f32;
                (TAU * phase).sin() * (-t / dec).exp()
            }
            56 => {
                let a = (TAU * 540.0 * t).sin().signum();
                let b = (TAU * 845.0 * t).sin().signum();
                0.4 * (a + b) * (-t / 0.20).exp()
            }
            49 | 55 | 57 => {
                let w = rng.bi();
                let mix = metal_808(t) * 0.6 + w * 0.5;
                hlp += a6000 * (mix - hlp);
                (mix - hlp) * 0.7 * (-t / 0.90).exp()
            }
            51 | 53 | 59 => {
                let w = rng.bi();
                let mix = metal_808(t) * 0.5 + w * 0.3;
                hlp += a6000 * (mix - hlp);
                (mix - hlp) * 0.6 * (-t / 0.50).exp()
                    + (TAU * 5200.0 * t).sin() * 0.20 * (-t / 0.30).exp()
            }
            _ => rng.bi() * (-t / 0.08).exp(),
        };
        out.push(s);
    }
    out
}

/// ADSR envelope with an sfxr-style punch boost on the initial transient.
fn adsr(env: &Adsr, n: usize, sr: u32) -> Signal {
    let Adsr { a, d, s, r, punch } = *env;
    let srf = sr as f32;
    let dur = n as f32 / srf;
    let rel_start = (dur - r).max(0.0);
    let punch_win = a + d;
    (0..n)
        .map(|i| {
            let t = i as f32 / srf;
            let mut v = if t < a {
                if a > 0.0 { t / a } else { 1.0 }
            } else if t < a + d {
                let p = if d > 0.0 { (t - a) / d } else { 1.0 };
                1.0 - (1.0 - s) * p
            } else if t < rel_start {
                s
            } else if r > 0.0 {
                let p = ((t - rel_start) / r).clamp(0.0, 1.0);
                s * (1.0 - p)
            } else {
                0.0
            };
            if punch > 0.0 && punch_win > 0.0 && t < punch_win {
                v *= 1.0 + punch * (1.0 - t / punch_win);
            }
            v
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn doc(json: &str) -> SoundDoc {
        serde_json::from_str(json).expect("deserialize")
    }

    fn rms(s: &[f32]) -> f32 {
        (s.iter().map(|x| x * x).sum::<f32>() / s.len() as f32).sqrt()
    }

    /// The determinism invariant for automation: a constant-value gain lane
    /// (every breakpoint = the static gain) renders byte-identically to a track
    /// with no automation at all — proving the automated path matches the fast
    /// path for constant values, so existing documents are unaffected.
    #[test]
    fn constant_gain_automation_is_byte_identical_to_static_gain() {
        let base = r#"{ "name":"t", "duration":0.4, "seed":1, "version":2,
            "root":{ "type":"tracks", "tracks":[ { "id":"a", "gain":0.8, "pan":-0.3,
              "node":{ "type":"mul", "inputs":[ {"type":"sine","freq":330},
                {"type":"env","a":0.01,"d":0.3,"s":0.4,"r":0.05} ] } } ] } }"#;
        let auto = r#"{ "name":"t", "duration":0.4, "seed":1, "version":2,
            "root":{ "type":"tracks", "tracks":[ { "id":"a", "gain":0.8, "pan":-0.3,
              "automation":[{"target":"gain","points":[{"t":0,"v":0.8},{"t":0.4,"v":0.8}]}],
              "node":{ "type":"mul", "inputs":[ {"type":"sine","freq":330},
                {"type":"env","a":0.01,"d":0.3,"s":0.4,"r":0.05} ] } } ] } }"#;
        let a = render_tracks(&doc(base)).unwrap();
        let b = render_tracks(&doc(auto)).unwrap();
        let bits = |s: &[f32]| s.iter().map(|x| x.to_bits()).collect::<Vec<_>>();
        assert_eq!(bits(&a.left), bits(&b.left), "left byte-identical");
        assert_eq!(bits(&a.right), bits(&b.right), "right byte-identical");
    }

    /// A gain ramp from 1 → 0 over the document makes the second half quieter
    /// than the first — automation actually rides the level.
    #[test]
    fn gain_automation_ramp_fades_the_track() {
        let d = doc(r#"{ "name":"t", "duration":1.0, "seed":1, "version":2,
            "root":{ "type":"tracks", "tracks":[ { "id":"a", "gain":1.0,
              "automation":[{"target":"gain","points":[{"t":0,"v":1.0},{"t":1.0,"v":0.0}]}],
              "node":{ "type":"sine", "freq":220 } } ] } }"#);
        let r = render_tracks(&d).unwrap();
        let half = r.left.len() / 2;
        let head = rms(&r.left[..half]);
        let tail = rms(&r.left[half..]);
        assert!(tail < head * 0.6, "ramp fades: head {head}, tail {tail}");
    }

    #[test]
    fn render_product_mid_is_the_track_bus_average() {
        let d = doc(r#"{ "name": "t", "duration": 0.05, "seed": 3, "version": 2,
                 "root": { "type": "tracks", "tracks": [
                    { "node": { "type": "sine", "freq": 220 }, "gain": 0.5 },
                    { "node": { "type": "noise" }, "gain": 0.5, "pan": 0.5 }
                 ] } }"#);
        let p = render_product(&d);
        let (l, r) = p.stereo.as_ref().expect("tracks doc carries the bus");
        assert_eq!(p.mono.len(), l.len());
        for i in [0usize, 100, 1000] {
            assert_eq!(p.mono[i], 0.5 * (l[i] + r[i]));
        }
        // Plain documents carry no pair; stereo treatment happens at write time.
        let plain =
            doc(r#"{ "name": "p", "duration": 0.05, "root": { "type": "sine", "freq": 220 } }"#);
        assert!(render_product(&plain).stereo.is_none());
    }

    #[test]
    fn v2_tracks_have_independent_rng_streams() {
        // Two docs that differ ONLY in track 0 (a sine consumes no RNG draws, a
        // noise consumes one per sample). Track 1 is hard-panned right, so the
        // right channel is its noise alone. Gains stay at 0.5 so the joint peak
        // limit never engages and the channels compare bit-for-bit.
        let mk = |first: &str, version: &str| {
            doc(&format!(
                r#"{{ "name": "t", "duration": 0.05, "seed": 7{version},
                     "root": {{ "type": "tracks", "tracks": [
                        {{ "node": {first}, "pan": -1.0, "gain": 0.5 }},
                        {{ "node": {{ "type": "noise" }}, "pan": 1.0, "gain": 0.5 }}
                     ] }} }}"#
            ))
        };
        let right = |d: &SoundDoc| render_tracks(d).unwrap().right;
        let sine = r#"{ "type": "sine", "freq": 440 }"#;
        let noise = r#"{ "type": "noise" }"#;
        // v2: editing track 0 never changes track 1's noise content.
        assert_eq!(
            right(&mk(sine, r#", "version": 2"#)),
            right(&mk(noise, r#", "version": 2"#))
        );
        // v1 (version omitted) keeps the legacy threaded stream — and with it
        // byte-identical replay of pre-versioning documents.
        assert_ne!(right(&mk(sine, "")), right(&mk(noise, "")));
    }

    #[test]
    fn layer_at_offset_shifts_and_truncates() {
        let d = doc(r#"{ "name": "t", "duration": 0.1, "version": 2,
                 "root": { "type": "tracks", "tracks": [
                    { "id": "late", "node": { "type": "sine", "freq": 440 },
                      "gain": 0.5, "at": 0.05 }
                 ] } }"#);
        let l = render_tracks(&d).unwrap().left;
        let head = rms(&l[..2000]);
        let tail = rms(&l[2300..]);
        assert!(head < 1e-6, "before `at` the bus is silent, rms {head}");
        assert!(tail > 0.1, "the layer plays from `at` on, rms {tail}");
        assert_eq!(l.len(), 4410); // shifted tail truncated at the doc edge
    }

    #[test]
    fn muted_layer_is_exactly_absent_in_v2() {
        let with_muted = doc(r#"{ "name": "t", "duration": 0.05, "seed": 9, "version": 2,
                 "root": { "type": "tracks", "tracks": [
                    { "id": "keep", "node": { "type": "noise" }, "gain": 0.5 },
                    { "id": "gone", "node": { "type": "noise" }, "gain": 0.5, "mute": true }
                 ] } }"#);
        let without = doc(r#"{ "name": "t", "duration": 0.05, "seed": 9, "version": 2,
                 "root": { "type": "tracks", "tracks": [
                    { "id": "keep", "node": { "type": "noise" }, "gain": 0.5 }
                 ] } }"#);
        // Id-keyed streams: muting/removing a sibling never re-grains "keep".
        let (a, b) = (
            render_tracks(&with_muted).unwrap(),
            render_tracks(&without).unwrap(),
        );
        assert_eq!((a.left, a.right), (b.left, b.right));
        // The muted layer still reports a (silent) stats row.
        assert!(a.layers[1].mute && a.layers[1].energy_pct == 0.0);
    }

    #[test]
    fn layer_stats_report_contribution() {
        let d = doc(r#"{ "name": "t", "duration": 0.05, "version": 2,
                 "root": { "type": "tracks", "tracks": [
                    { "id": "loud", "node": { "type": "sine", "freq": 220 }, "gain": 0.8 },
                    { "id": "quiet", "node": { "type": "sine", "freq": 330 }, "gain": 0.2 }
                 ] } }"#);
        let tr = render_tracks(&d).unwrap();
        assert_eq!(tr.layers.len(), 2);
        assert_eq!(tr.layers[0].id, "loud");
        assert_eq!(tr.layers[1].id, "quiet");
        // 0.8 vs 0.2 gain ⇒ a 16:1 energy split.
        assert!(tr.layers[0].energy_pct > 90.0, "{:?}", tr.layers);
        assert!(tr.layers[1].energy_pct < 10.0, "{:?}", tr.layers);
        let total: f32 = tr.layers.iter().map(|l| l.energy_pct).sum();
        assert!((total - 100.0).abs() < 0.1);
        // dB sanity: the loud layer peaks ~12 dB above the quiet one.
        let gap = tr.layers[0].peak_dbfs - tr.layers[1].peak_dbfs;
        assert!((gap - 12.04).abs() < 0.2, "gap {gap}");
    }

    #[test]
    fn wrapping_a_plain_root_as_a_compensated_layer_is_level_neutral() {
        let plain = doc(r#"{ "name": "p", "duration": 0.05,
                 "root": { "type": "mul", "inputs": [
                    { "type": "sine", "freq": 330 },
                    { "type": "env", "a": 0.0, "d": 0.04, "s": 0.0, "r": 0.0 } ] } }"#);
        let wrapped = doc(r#"{ "name": "p", "duration": 0.05, "version": 2,
                 "root": { "type": "tracks", "tracks": [
                    { "id": "p", "gain": 1.4142135,
                      "node": { "type": "mul", "inputs": [
                        { "type": "sine", "freq": 330 },
                        { "type": "env", "a": 0.0, "d": 0.04, "s": 0.0, "r": 0.0 } ] } }
                 ] } }"#);
        let a = render(&plain);
        let b = render(&wrapped); // mid of the wrapped bus
        let max_diff = a
            .iter()
            .zip(&b)
            .fold(0.0f32, |m, (x, y)| m.max((x - y).abs()));
        assert!(
            max_diff < 1e-6,
            "wrap must be level-neutral, diff {max_diff}"
        );
    }

    #[test]
    fn render_is_deterministic() {
        let d = doc(r#"{ "name": "n", "duration": 0.1, "seed": 7,
                 "root": { "type": "noise" } }"#);
        assert_eq!(render(&d), render(&d)); // same graph + seed ⇒ same bytes
        let mut d2 = d.clone();
        d2.seed = 8;
        assert_ne!(render(&d), render(&d2)); // different seed ⇒ different noise
    }

    #[test]
    fn sine_has_expected_length_and_level() {
        let d = doc(r#"{ "name": "n", "duration": 0.1,
                 "root": { "type": "sine", "freq": 440 } }"#);
        let s = render(&d);
        assert_eq!(s.len(), 4410);
        let peak = s.iter().fold(0.0f32, |m, &x| m.max(x.abs()));
        assert!(peak > 0.9 && peak <= 1.0);
        assert!((rms(&s) - 0.7).abs() < 0.05); // sine RMS = peak/√2
    }

    #[test]
    fn envelope_gates_the_oscillator() {
        let d = doc(
            r#"{ "name": "n", "duration": 0.2, "root": { "type": "mul", "inputs": [
                { "type": "square", "freq": 440 },
                { "type": "env", "a": 0.0, "d": 0.05, "s": 0.0, "r": 0.0 }
            ] } }"#,
        );
        let s = render(&d);
        // Loud at the start, silent at the end (envelope fully decayed).
        let head = rms(&s[..2205]);
        let tail = rms(&s[s.len() - 2205..]);
        assert!(head > 0.1, "head should be audible, rms {head}");
        assert!(tail < 1e-3, "tail should be silent, rms {tail}");
    }

    #[test]
    fn slide_descends_pitch() {
        // A 880→110 Hz exponential slide: zero crossings in the first half
        // outnumber those in the second half.
        let d = doc(r#"{ "name": "n", "duration": 0.5, "root": { "type": "sine",
                 "freq": { "slide": { "from": 880, "to": 110, "secs": 0.5, "curve": "exp" } } } }"#);
        let s = render(&d);
        let crossings = |w: &[f32]| w.windows(2).filter(|p| p[0] * p[1] < 0.0).count();
        let (a, b) = s.split_at(s.len() / 2);
        assert!(crossings(a) > crossings(b) * 2);
    }

    #[test]
    fn seq_places_notes_on_the_grid() {
        // 120 bpm, 4 steps/beat ⇒ 0.125 s per step. A note at step 2 starts at
        // 0.25 s; everything before is silence.
        let d = doc(r#"{ "name": "n", "duration": 0.6, "root": { "type": "seq",
                 "bpm": 120, "wave": "square",
                 "env": { "d": 0.1 },
                 "notes": [ { "step": 2, "len": 2, "pitch": "C4" } ] } }"#);
        let s = render(&d);
        let pre = rms(&s[..(0.24 * 44_100.0) as usize]);
        let post = rms(&s[(0.26 * 44_100.0) as usize..(0.35 * 44_100.0) as usize]);
        assert!(pre < 1e-4, "before the note: silence, rms {pre}");
        assert!(post > 0.05, "during the note: audible, rms {post}");
    }

    /// Brightness proxy: energy of the first difference relative to the
    /// signal (high-frequency content differentiates to larger steps).
    fn brightness(s: &[f32]) -> f32 {
        let diff: f32 = s.windows(2).map(|w| (w[1] - w[0]).powi(2)).sum();
        let total: f32 = s.iter().map(|x| x * x).sum();
        diff / total.max(1e-12)
    }

    #[test]
    fn lowpass_darkens_highpass_brightens() {
        let noise = r#"{ "type": "noise" }"#;
        let plain = doc(&format!(
            r#"{{ "name": "n", "duration": 0.2, "root": {noise} }}"#
        ));
        let lp = doc(&format!(
            r#"{{ "name": "n", "duration": 0.2, "root": {{ "type": "chain", "stages": [
                {noise}, {{ "type": "lowpass", "cutoff": 500 }} ] }} }}"#
        ));
        let hp = doc(&format!(
            r#"{{ "name": "n", "duration": 0.2, "root": {{ "type": "chain", "stages": [
                {noise}, {{ "type": "highpass", "cutoff": 5000 }} ] }} }}"#
        ));
        let b_plain = brightness(&render(&plain));
        assert!(brightness(&render(&lp)) < b_plain * 0.5, "lowpass darkens");
        assert!(
            brightness(&render(&hp)) > b_plain * 1.1,
            "highpass brightens"
        );
    }

    #[test]
    fn chain_processors_transform_in_series() {
        // sine → gain 0.25: the processor scales the running signal.
        let d = doc(
            r#"{ "name": "n", "duration": 0.05, "root": { "type": "chain", "stages": [
                { "type": "sine", "freq": 440 },
                { "type": "gain", "amount": 0.25 }
            ] } }"#,
        );
        let s = render(&d);
        let peak = s.iter().fold(0.0f32, |m, &x| m.max(x.abs()));
        assert!((peak - 0.25).abs() < 0.01);
    }

    #[test]
    fn bitcrush_quantizes_amplitude() {
        // The gain stage keeps the crushed peak under the output ceiling so the
        // safety limit stays out of the way and the levels survive untouched.
        let d = doc(
            r#"{ "name": "n", "duration": 0.05, "root": { "type": "chain", "stages": [
                { "type": "sine", "freq": 100 },
                { "type": "gain", "amount": 0.5 },
                { "type": "bitcrush", "bits": 2 }
            ] } }"#,
        );
        let s = render(&d);
        // 2 bits ⇒ amplitudes land on multiples of 0.5.
        for x in &s {
            let nearest = (x / 0.5).round() * 0.5;
            assert!((x - nearest).abs() < 1e-4, "{x} not on a 2-bit level");
        }
    }

    #[test]
    fn drive_hard_clips_to_unit_range() {
        let d = doc(
            r#"{ "name": "n", "duration": 0.05, "root": { "type": "chain", "stages": [
                { "type": "sine", "freq": 440 },
                { "type": "drive", "amount": 10, "shape": "hard" }
            ] } }"#,
        );
        // Heavy drive into a hard clip ⇒ near-square at the ceiling.
        let s = render(&d);
        let clipped = s.iter().filter(|x| x.abs() > 0.95).count();
        assert!(clipped > s.len() / 2);
    }

    #[test]
    fn drive_antiderivative_matches_its_curve() {
        // F'(x) ≈ drive_curve(x): a central difference of the antiderivative
        // must reproduce the waveshaper, the property ADAA relies on.
        let h = 1e-3f32;
        for shape in [DriveShape::Tanh, DriveShape::Hard, DriveShape::Fold] {
            for &x in &[-3.5f32, -1.2, -0.4, 0.0, 0.6, 1.5, 4.2] {
                let num =
                    (drive_antideriv(x + h, shape) - drive_antideriv(x - h, shape)) / (2.0 * h);
                let exact = drive_curve(x, shape);
                assert!(
                    (num - exact).abs() < 5e-3,
                    "{shape:?} at x={x}: dF/dx={num} vs f={exact}"
                );
            }
        }
    }

    #[test]
    fn adaa_engages_only_under_engine_1() {
        // Identical bright-tone-into-fold graph at engine 0 vs engine 1.
        let mk = |engine: u32| {
            doc(&format!(
                r#"{{ "name": "n", "duration": 0.1, "engine": {engine},
                     "root": {{ "type": "chain", "stages": [
                        {{ "type": "sine", "freq": 3000 }},
                        {{ "type": "drive", "amount": 6, "shape": "fold" }}
                     ] }} }}"#
            ))
        };
        let legacy = render(&mk(0));
        let aa = render(&mk(1));
        // Engine 0 must be byte-identical to the original pointwise curve.
        let n = legacy.len();
        let mut rng = Rng::new(0);
        let reference = {
            let sine = render_node(
                &Node::Sine {
                    freq: Value::Const(3000.0),
                },
                n,
                44_100,
                &mut rng,
                0,
                0,
            );
            let amt = eval_value(&Value::Const(6.0), n, 44_100);
            let raw: Vec<f32> = sine
                .iter()
                .zip(amt)
                .map(|(x, a)| drive_curve(a.max(0.0) * x, DriveShape::Fold))
                .collect();
            // render() applies the default peak-limit; mirror it.
            let mut r = raw;
            peak_limit(&mut [&mut r]);
            r
        };
        assert_eq!(legacy, reference, "engine 0 drive must stay bit-exact");
        // Engine 1 genuinely changes the signal …
        assert_ne!(legacy, aa, "engine 1 must apply ADAA");
        // … by band-limiting it: the mean-square of the sample-to-sample
        // difference (a high-frequency-energy proxy) drops, because the
        // inharmonic foldback that ADAA removes is the spikiest content.
        let diff_energy = |s: &[f32]| -> f32 {
            s.windows(2).map(|w| (w[1] - w[0]).powi(2)).sum::<f32>() / s.len() as f32
        };
        assert!(
            diff_energy(&aa) < diff_energy(&legacy),
            "ADAA should reduce HF energy: aa={} legacy={}",
            diff_energy(&aa),
            diff_energy(&legacy)
        );
    }

    #[cfg(feature = "analysis")]
    #[test]
    fn adaa_lowers_off_harmonic_energy_for_a_folded_tone() {
        // A 2500 Hz sine folded hard: its true harmonics sit on the 2500 Hz
        // grid, but the un-band-limited version folds high harmonics back to
        // OFF-grid frequencies. ADAA suppresses that foldback, so the
        // analyzer's `inharmonicity` meter reads lower — the feedback loop can
        // SEE the fix. (The relationship is signal-dependent in general; this
        // is a clear, reproducible case, not a universal law.)
        let mk = |engine: u32| {
            doc(&format!(
                r#"{{ "name": "n", "duration": 0.3, "engine": {engine},
                     "root": {{ "type": "chain", "stages": [
                        {{ "type": "sine", "freq": 2500 }},
                        {{ "type": "drive", "amount": 8, "shape": "fold" }}
                     ] }} }}"#
            ))
        };
        let inharm = |d: &SoundDoc| crate::analysis::stats(&render(d), 44_100).inharmonicity;
        let legacy = inharm(&mk(0));
        let aa = inharm(&mk(1));
        assert!(
            aa < legacy - 0.1,
            "ADAA should clearly lower off-harmonic energy: aa={aa} legacy={legacy}"
        );
    }

    #[test]
    fn impact_is_a_short_unit_area_pulse() {
        let d = doc(r#"{ "name": "n", "duration": 0.2, "engine": 1,
                 "root": { "type": "impact", "hardness": 0.5, "velocity": 1.0 } }"#);
        let s = render(&d);
        // The pulse is confined to the first ~10 ms; the rest is silence.
        let head = (0.02 * 44_100.0) as usize;
        assert!(
            s[head..].iter().all(|x| x.abs() < 1e-6),
            "impact must be a short burst"
        );
        // Unit area (× velocity 1) — the level guarantee a modal bank rings to.
        let area: f32 = s.iter().sum();
        assert!(
            (area - 1.0).abs() < 0.05,
            "impact area ≈ velocity, got {area}"
        );
    }

    #[test]
    fn modal_bank_rings_at_its_mode_and_decays() {
        let d = doc(r#"{ "name": "n", "duration": 0.4, "engine": 1,
                 "root": { "type": "chain", "stages": [
                    { "type": "impact", "hardness": 0.8, "velocity": 1.0 },
                    { "type": "modal", "modes": [ { "freq": 1000, "decay": 0.3, "gain": 1.0 } ] }
                 ] } }"#);
        let s = render(&d);
        // Usable level (not the −44 dBFS the un-normalised first cut produced).
        let peak = s.iter().fold(0.0f32, |m, x| m.max(x.abs()));
        assert!(peak > 0.1, "modal ring too quiet: peak {peak}");
        // Rings at the mode frequency: count zero crossings over a steady
        // window and convert to Hz (a single mode is a clean decaying sine).
        let (a, b) = ((0.05 * 44_100.0) as usize, (0.15 * 44_100.0) as usize);
        let win = &s[a..b];
        let zc = win
            .windows(2)
            .filter(|w| (w[0] <= 0.0) != (w[1] <= 0.0))
            .count();
        let hz = zc as f32 / 2.0 / 0.1;
        assert!((hz - 1000.0).abs() < 80.0, "expected ≈1000 Hz, got {hz}");
        // Decays: the tail is quieter than the body.
        assert!(
            rms(&s[s.len() / 2..]) < rms(&s[..s.len() / 2]),
            "modal must decay"
        );
    }

    #[test]
    fn rand_modulator_is_self_seeded_and_bounded() {
        let v = |seed: u64| {
            Value::Modulated(Modulator::Rand {
                from: 200.0,
                to: 800.0,
                rate: 5.0,
                seed,
            })
        };
        // Deterministic from its own fields only — no shared-stream coupling,
        // so a sibling edit elsewhere in the graph can never shift it.
        let a = eval_value(&v(1), 4410, 44_100);
        assert_eq!(a, eval_value(&v(1), 4410, 44_100));
        // A different seed decorrelates the walk.
        assert_ne!(a, eval_value(&v(2), 4410, 44_100));
        // The walk stays inside [from, to].
        assert!(a.iter().all(|&x| (200.0..=800.0).contains(&x)));
    }

    #[test]
    fn dust_is_sparse_and_deterministic() {
        let mk = || {
            doc(r#"{ "name": "n", "duration": 1.0, "engine": 1, "seed": 4,
                     "root": { "type": "dust", "density": 20, "decay": 0.0 } }"#)
        };
        let a = render(&mk());
        assert_eq!(a, render(&mk()), "dust must be deterministic");
        // ~20 events/sec over 1 s; decay 0 ⇒ one nonzero sample per event.
        let events = a.iter().filter(|&&x| x.abs() > 1e-6).count();
        assert!(
            (5..60).contains(&events),
            "expected ≈20 sparse events, got {events}"
        );
    }

    #[test]
    fn compressor_attenuates_above_threshold() {
        // A 0 dBFS sine through threshold −20 dB, ratio 4:1 settles at a steady
        // gain of −(0 − (−20))·(1 − 1/4) = −15 dB.
        let wet = doc(
            r#"{ "name": "n", "duration": 0.3, "root": { "type": "chain", "stages": [
                { "type": "sine", "freq": 440 },
                { "type": "compress", "threshold": -20, "ratio": 4 }
            ] } }"#,
        );
        let dry =
            doc(r#"{ "name": "n", "duration": 0.3, "root": { "type": "sine", "freq": 440 } }"#);
        // Skip the attack transient, measure the settled tail.
        let tail = |s: Vec<f32>| rms(&s[s.len() / 2..]);
        let ratio = tail(render(&wet)) / tail(render(&dry));
        let db = 20.0 * ratio.log10();
        assert!((db + 15.0).abs() < 2.0, "expected ≈ −15 dB, got {db:.1} dB");
    }

    #[test]
    fn loop_body_is_region_minus_crossfade() {
        let sr = 1000u32;
        let samples = vec![0.5f32; 1000]; // 1 s
        // Region [0.2, 0.8) = 600 samples, crossfade 0.1 s = 100 ⇒ body 500.
        let out = make_loop_buffer(&samples, sr, 0.2, Some(0.8), 0.1);
        assert_eq!(out.len(), 500);
        // Degenerate inputs fall back gracefully.
        assert_eq!(
            make_loop_buffer(&samples, sr, 0.9, Some(0.1), 0.1).len(),
            1000
        );
        assert_eq!(make_loop_buffer(&samples, sr, 0.0, None, 0.0).len(), 1000);
    }

    #[test]
    fn looped_render_has_a_quiet_seam() {
        // A sustained noise bed rendered as a loop: the wrap-around jump should
        // be far below the raw signal's sample-to-sample movement.
        let d = doc(r#"{ "name": "n", "duration": 1.0, "seed": 3,
                 "playback": { "mode": "loop", "crossfade_secs": 0.25 },
                 "root": { "type": "chain", "stages": [
                    { "type": "noise" }, { "type": "lowpass", "cutoff": 800 } ] } }"#);
        let s = render(&d);
        assert!(s.len() < 44_100); // body shortened by the crossfade
        assert!(loop_seam_db(&s) < -20.0, "seam {} dB", loop_seam_db(&s));
    }

    #[test]
    fn normalize_hits_the_loudness_target() {
        let d = doc(r#"{ "name": "n", "duration": 0.5,
                 "normalize": { "target_lufs": -20, "ceiling_dbtp": -1 },
                 "root": { "type": "chain", "stages": [
                    { "type": "sine", "freq": 440 }, { "type": "gain", "amount": 0.05 } ] } }"#);
        let s = render(&d);
        let lufs = loudness_lufs(&s);
        assert!((lufs + 20.0).abs() < 1.5, "got {lufs} LUFS");
        // True peak respects the −1 dBTP ceiling (small estimation slack).
        assert!(crate::dsp::dbfs(true_peak(&s)) <= -0.9);
    }

    #[test]
    fn stereoize_modes_behave() {
        let d = doc(r#"{ "name": "n", "duration": 0.1, "root": { "type": "noise" } }"#);
        let mono = render(&d);
        let (l, r) = stereoize(&mono, Stereo::Mono, 44_100);
        assert_eq!(l, r);
        let (l, r) = stereoize(&mono, Stereo::Wide { amount: 0.8 }, 44_100);
        assert_ne!(l, r); // decorrelated channels differ...
        let mid_rms = rms(&l
            .iter()
            .zip(&r)
            .map(|(a, b)| (a + b) / 2.0)
            .collect::<Vec<_>>());
        assert!(mid_rms > 0.1); // ...but the mid (mono sum) survives
        let (l, r) = stereoize(&mono, Stereo::Haas { ms: 10.0, pan: 1.0 }, 44_100);
        let delay = (0.010 * 44_100.0) as usize;
        assert_eq!(l[delay..delay + 100], mono[..100]); // left trails by 10 ms
        assert_eq!(r[..100], mono[..100]); // right leads
    }

    #[test]
    fn fm_seq_strikes_bright_then_mellows() {
        // One sustained fm note: the decaying modulation index makes the
        // attack brighter than the tail — the hammer-strike signature.
        let d = doc(r#"{ "name": "n", "duration": 1.0, "root": { "type": "seq",
                 "bpm": 60, "steps_per_beat": 1, "wave": "fm",
                 "fm_ratio": 1.0, "fm_index": 6, "fm_strike": 0.15,
                 "env": { "d": 0.9, "s": 0.5 },
                 "notes": [ { "step": 0, "len": 1, "pitch": "A3" } ] } }"#);
        let s = render(&d);
        assert!(rms(&s) > 0.05, "fm note audible");
        let third = s.len() / 3;
        assert!(
            brightness(&s[..third]) > brightness(&s[2 * third..]) * 1.5,
            "strike should be brighter than the tail"
        );
    }

    #[test]
    fn pluck_seq_rings_and_decays_deterministically() {
        let json = r#"{ "name": "n", "duration": 1.2, "seed": 9, "root": { "type": "seq",
            "bpm": 60, "steps_per_beat": 1, "wave": "pluck", "pluck_decay": 0.995,
            "env": { "d": 0.1, "s": 1.0 },
            "notes": [ { "step": 0, "len": 1, "pitch": "A3" } ] } }"#;
        let s = render(&doc(json));
        let half = s.len() / 2;
        assert!(rms(&s[..half]) > 0.05, "pluck audible");
        assert!(
            rms(&s[half..]) < rms(&s[..half]) * 0.5,
            "string decays naturally"
        );
        // Same seed ⇒ identical string; different seed ⇒ different noise burst.
        assert_eq!(s, render(&doc(json)));
        let mut other = doc(json);
        other.seed = 10;
        assert_ne!(s, render(&other));
    }

    #[test]
    fn piano_bass_rings_longer_than_treble() {
        let note = |pitch: &str| {
            let d = doc(&format!(
                r#"{{ "name": "n", "duration": 2.0, "root": {{ "type": "seq",
                     "bpm": 60, "steps_per_beat": 1, "wave": "piano",
                     "env": {{ "a": 0.002, "s": 1.0, "r": 0.1 }},
                     "notes": [ {{ "step": 0, "len": 2, "pitch": "{pitch}" }} ] }} }}"#
            ));
            render(&d)
        };
        let tail_ratio = |s: &[f32]| {
            let q = s.len() / 4;
            rms(&s[2 * q..3 * q]) / rms(&s[..q]).max(1e-9)
        };
        let bass = note("A1");
        let treble = note("A5");
        assert!(rms(&bass) > 0.02 && rms(&treble) > 0.005, "both audible");
        assert!(
            tail_ratio(&bass) > tail_ratio(&treble) * 1.5,
            "bass sustains, treble dies: {} vs {}",
            tail_ratio(&bass),
            tail_ratio(&treble)
        );
    }

    #[test]
    fn engine3_piano_is_a_distinct_richer_voice() {
        let seq = |engine: u32, pitch: &str| {
            doc(&format!(
                r#"{{ "name": "n", "duration": 2.0, "engine": {engine}, "root": {{ "type": "seq",
                     "bpm": 60, "steps_per_beat": 1, "wave": "piano",
                     "env": {{ "a": 0.002, "s": 1.0, "r": 0.1 }},
                     "notes": [ {{ "step": 0, "len": 2, "pitch": "{pitch}" }} ] }} }}"#
            ))
        };
        let peak = |s: &[f32]| s.iter().fold(0.0f32, |m, &x| m.max(x.abs()));
        let legacy = render(&seq(2, "C4"));
        let v3 = render(&seq(3, "C4"));
        // The engine-3 model is a genuinely different (and non-clipping) waveform;
        // the legacy engine-2 voice is untouched by the upgrade.
        assert!(peak(&v3) > 0.05 && peak(&v3) < 1.1, "audible, not clipping");
        assert_ne!(legacy, v3, "engine 3 upgrades the piano voice");
        // The pitch-dependent ring survives: bass sustains, treble dies fast.
        let tail = |s: &[f32]| {
            let q = s.len() / 4;
            rms(&s[2 * q..3 * q]) / rms(&s[..q]).max(1e-9)
        };
        let bass = render(&seq(3, "A1"));
        let treble = render(&seq(3, "A5"));
        assert!(
            tail(&bass) > tail(&treble) * 1.5,
            "engine-3 bass rings longer than treble: {} vs {}",
            tail(&bass),
            tail(&treble)
        );
    }

    #[test]
    fn engine3_piano_tone_knobs_default_to_the_concert_grand() {
        // Omitting the piano_* keys must render byte-identically to setting them
        // at their documented defaults — the byte-safe contract for the knobs.
        let bare = r#"{ "name":"n", "duration":1.0, "engine":3, "root": { "type":"seq",
            "bpm":60, "steps_per_beat":1, "wave":"piano", "env": { "a":0.002, "s":1.0, "r":0.1 },
            "notes": [ { "step":0, "len":1, "pitch":"C4" } ] } }"#;
        let defaults = r#"{ "name":"n", "duration":1.0, "engine":3, "root": { "type":"seq",
            "bpm":60, "steps_per_beat":1, "wave":"piano", "env": { "a":0.002, "s":1.0, "r":0.1 },
            "piano_hammer":1.0, "piano_strike":0.125, "piano_inharm":1.0, "piano_detune":1.0, "piano_decay":1.0,
            "notes": [ { "step":0, "len":1, "pitch":"C4" } ] } }"#;
        assert_eq!(
            render(&doc(bare)),
            render(&doc(defaults)),
            "the tone-knob defaults reproduce the grand bit-for-bit"
        );
    }

    #[test]
    fn engine3_piano_variants_are_spectrally_distinct() {
        let piano = |extra: &str| {
            doc(&format!(
                r#"{{ "name":"n", "duration":1.5, "engine":3, "root": {{ "type":"seq",
                    "bpm":60, "steps_per_beat":1, "wave":"piano", "env": {{ "a":0.002, "s":1.0, "r":0.1 }},
                    {extra}
                    "notes": [ {{ "step":0, "len":1, "pitch":"C4", "gain":0.9 }} ] }} }}"#
            ))
        };
        let grand = render(&piano(""));
        let felt = render(&piano(
            r#""piano_hammer":0.35, "piano_strike":0.16, "piano_decay":0.8,"#,
        ));
        let honky = render(&piano(r#""piano_detune":12.0, "piano_inharm":1.7,"#));
        assert_ne!(grand, felt, "felt is a different waveform");
        assert_ne!(grand, honky, "honky-tonk is a different waveform");
        // Felt's soft hammer removes upper partials — less high-frequency energy
        // (a first-difference sum is a crude high-pass).
        let hf = |s: &[f32]| s.windows(2).map(|w| (w[1] - w[0]).abs()).sum::<f32>();
        assert!(
            hf(&felt) < hf(&grand),
            "felt is darker than the grand: {} vs {}",
            hf(&felt),
            hf(&grand)
        );
    }

    #[test]
    fn kit_styles_are_distinct_bounded_and_default_to_classic() {
        let kit = |style: &str| {
            let key = if style.is_empty() {
                String::new()
            } else {
                format!(r#""kit":"{style}", "#)
            };
            doc(&format!(
                r#"{{ "name":"n", "duration":1.0, "engine":3, "root": {{ "type":"seq",
                    "bpm":120, "steps_per_beat":4, "wave":"kit", "env": {{ "a":0.001, "s":1.0, "r":0.05 }}, {key}
                    "notes": [ {{"step":0,"len":1,"pitch":"midi:36"}}, {{"step":2,"len":1,"pitch":"midi:38"}},
                               {{"step":4,"len":1,"pitch":"midi:42"}}, {{"step":6,"len":1,"pitch":"midi:49"}} ] }} }}"#
            ))
        };
        let peak = |s: &[f32]| s.iter().fold(0.0f32, |m, &x| m.max(x.abs()));
        let classic = render(&kit(""));
        let acoustic = render(&kit("acoustic"));
        let electronic = render(&kit("electronic"));
        let eight = render(&kit("808"));
        for (name, s) in [
            ("acoustic", &acoustic),
            ("electronic", &electronic),
            ("808", &eight),
        ] {
            assert!(s.iter().all(|x| x.is_finite()), "{name}: no NaN/inf");
            assert!(
                peak(s) > 0.05 && peak(s) < 2.5,
                "{name} audible+bounded: {}",
                peak(s)
            );
            assert_ne!(&classic, s, "{name} differs from the classic kit");
        }
        assert_ne!(acoustic, electronic, "acoustic and electronic differ");
        assert_ne!(electronic, eight, "electronic and 808 differ");
        // Omitting `kit` renders identically to selecting `classic`.
        assert_eq!(
            render(&kit("")),
            render(&kit("classic")),
            "default == classic"
        );
    }

    #[test]
    fn bass_tone_knobs_default_to_the_current_voice_and_variants_differ() {
        let bass = |extra: &str| {
            doc(&format!(
                r#"{{ "name":"n", "duration":1.5, "engine":3, "root": {{ "type":"seq",
                    "bpm":90, "steps_per_beat":2, "wave":"bass", "env": {{ "a":0.005, "d":0.1, "s":0.9, "r":0.12 }},
                    {extra}
                    "notes": [ {{"step":0,"len":4,"pitch":"E1","gain":0.9}} ] }} }}"#
            ))
        };
        // Byte-safe: omitting the bass_* keys == setting them at their defaults.
        let bare = render(&bass(""));
        let defaults = render(&bass(
            r#""bass_cutoff":250.0,"bass_env":700.0,"bass_env_vel":1100.0,"bass_decay":0.15,"bass_click":0.0,"bass_body":0.7,"bass_sub":0.45,"bass_sub_ratio":1.0,"bass_drive":0.0,"bass_body_decay":2.0,"#,
        ));
        assert_eq!(bare, defaults, "bass defaults reproduce the current voice");
        // A driven synth-bass variant is a different, well-formed waveform.
        let synth = render(&bass(
            r#""bass_cutoff":600.0,"bass_drive":0.35,"bass_sub_ratio":0.5,"bass_body_decay":6.0,"#,
        ));
        assert!(synth.iter().all(|x| x.is_finite()));
        assert_ne!(bare, synth, "synth bass differs from finger");
        // The octave-down sub (ratio 0.5) puts real energy below the fundamental.
        let octave = render(&bass(r#""bass_sub":0.9,"bass_sub_ratio":0.5,"#));
        assert_ne!(bare, octave, "octave-down sub changes the voice");
    }

    #[test]
    fn guitar_tone_stages_default_to_identity_and_variants_differ() {
        let pluck = |extra: &str| {
            doc(&format!(
                r#"{{ "name":"n", "duration":1.2, "engine":3, "seed":3, "root": {{ "type":"seq",
                    "bpm":90, "steps_per_beat":2, "wave":"pluck", "pluck_decay":0.96, "env": {{ "a":0.001, "s":1.0, "r":0.2 }},
                    {extra}
                    "notes": [ {{"step":0,"len":4,"pitch":"E3","gain":0.9}} ] }} }}"#
            ))
        };
        // Byte-safe: omitting the stages == setting them at their identity defaults.
        let bare = render(&pluck(""));
        let defaults = render(&pluck(
            r#""pluck_body":0.0,"pluck_pick":0.0,"pluck_tone":0.0,"#,
        ));
        assert_eq!(bare, defaults, "the pluck tone stages default to a no-op");
        // A bodied, dark nylon variant is a different, well-formed waveform.
        let nylon = render(&pluck(
            r#""pluck_body":0.55,"pluck_pick":0.05,"pluck_tone":-0.35,"#,
        ));
        assert!(nylon.iter().all(|x| x.is_finite()));
        assert_ne!(bare, nylon, "nylon body/tone/pick change the voice");
    }

    fn one_note(wave: &str, pitch: &str, secs: f32) -> Vec<f32> {
        let d = doc(&format!(
            r#"{{ "name": "n", "duration": {secs}, "root": {{ "type": "seq",
                 "bpm": 60, "steps_per_beat": 1, "wave": "{wave}",
                 "env": {{ "a": 0.002, "s": 1.0, "r": 0.05 }},
                 "notes": [ {{ "step": 0, "len": {len}, "pitch": "{pitch}" }} ] }} }}"#,
            len = secs.ceil() as u32,
        ));
        render(&d)
    }

    #[test]
    fn epiano_tine_pings_then_mellows() {
        let s = one_note("epiano", "A3", 1.0);
        assert!(rms(&s) > 0.05, "epiano audible");
        let q = s.len() / 4;
        assert!(brightness(&s[..q]) > brightness(&s[3 * q..]) * 1.3);
    }

    #[test]
    fn organ_sustains_while_held() {
        let s = one_note("organ", "C3", 1.0);
        assert!(rms(&s) > 0.1, "organ audible");
        let q = s.len() / 4;
        // No natural decay: the last quarter holds level with the second.
        let (mid, tail) = (rms(&s[q..2 * q]), rms(&s[3 * q..]));
        assert!(tail > mid * 0.7, "organ holds: {mid} -> {tail}");
    }

    #[test]
    fn strings_swell_in_slowly() {
        let s = one_note("strings", "A3", 1.0);
        assert!(rms(&s) > 0.05, "strings audible");
        let ms50 = 44_100 / 20;
        // The bow swell: the first 50 ms is much quieter than the body.
        assert!(rms(&s[..ms50]) < rms(&s[ms50 * 6..ms50 * 8]) * 0.6);
    }

    #[test]
    fn bass_is_darker_than_a_raw_saw() {
        let b = one_note("bass", "E2", 0.5);
        let saw = one_note("sawtooth", "E2", 0.5);
        assert!(rms(&b) > 0.05, "bass audible");
        assert!(
            brightness(&b) < brightness(&saw) * 0.5,
            "bass is filtered dark"
        );
    }

    #[test]
    fn tracks_pan_places_instruments_on_the_stage() {
        let d = doc(
            r#"{ "name": "n", "duration": 0.2, "root": { "type": "tracks", "tracks": [
                { "pan": -1.0, "node": { "type": "sine", "freq": 440 } },
                { "pan":  1.0, "gain": 0.5, "node": { "type": "sine", "freq": 660 } }
            ] } }"#,
        );
        assert_eq!(d.validate(), Ok(()));
        let tr = render_tracks(&d).unwrap();
        let (l, r) = (tr.left, tr.right);
        // Hard-left 440 dominates L; hard-right (at half gain) is alone on R.
        assert!(
            rms(&l) > rms(&r) * 1.5,
            "left louder: {} vs {}",
            rms(&l),
            rms(&r)
        );
        let zero_crossings = |s: &[f32]| s.windows(2).filter(|w| w[0] * w[1] < 0.0).count();
        // R carries only the 660 Hz track ⇒ more crossings per second.
        assert!(zero_crossings(&r) > zero_crossings(&l));
        // The public mono render is the mid of the same bus.
        let mid = render(&d);
        assert!((mid[1000] - 0.5 * (l[1000] + r[1000])).abs() < 1e-6);
    }

    #[test]
    fn tracks_master_reverb_decorrelates_the_channels() {
        let d = doc(
            r#"{ "name": "n", "duration": 0.5, "root": { "type": "tracks",
                 "tracks": [ { "node": { "type": "mul", "inputs": [
                     { "type": "sine", "freq": 440 },
                     { "type": "env", "d": 0.1 } ] } } ],
                 "master": [ { "type": "reverb", "room": 0.6, "mix": 0.4 } ] } }"#,
        );
        let tr = render_tracks(&d).unwrap();
        let (l, r) = (tr.left, tr.right);
        assert_ne!(l, r, "spread reverb gives each side its own tail");
        // And with a duck in the master, both channels stay deterministic.
        let d2 = doc(
            r#"{ "name": "n", "duration": 0.5, "seed": 3, "root": { "type": "tracks",
                 "tracks": [ { "node": { "type": "noise" } } ],
                 "master": [ { "type": "duck", "amount": 0.7,
                   "trigger": { "type": "seq", "bpm": 120, "steps_per_beat": 1,
                     "wave": "kit", "env": { "s": 1 },
                     "notes": [ { "step": 0, "len": 1, "pitch": "midi:36" } ] } } ] } }"#,
        );
        let a = render_tracks(&d2).unwrap();
        let b = render_tracks(&d2).unwrap();
        assert_eq!(a, b, "stereo master bus renders are byte-stable");
    }

    #[test]
    fn tracks_validation_guards_the_console() {
        let nested = doc(r#"{ "name": "n", "root": { "type": "mix", "inputs": [
                { "type": "tracks", "tracks": [ { "node": { "type": "noise" } } ] }
            ] } }"#);
        assert!(nested.validate().unwrap_err().contains("root"));
        let bad_master = doc(r#"{ "name": "n", "root": { "type": "tracks",
                 "tracks": [ { "node": { "type": "noise" } } ],
                 "master": [ { "type": "sine", "freq": 440 } ] } }"#);
        assert!(bad_master.validate().unwrap_err().contains("master"));
        let bad_pan = doc(r#"{ "name": "n", "root": { "type": "tracks",
                 "tracks": [ { "pan": 2.0, "node": { "type": "noise" } } ] } }"#);
        assert!(bad_pan.validate().unwrap_err().contains("pan"));
    }

    #[test]
    fn sampler_requires_a_real_soundfont_path() {
        let d = doc(r#"{ "name": "n", "duration": 0.5, "root": { "type": "seq",
                 "bpm": 120, "wave": "sampler", "sf2": "/no/such/font.sf2",
                 "env": { "s": 1 },
                 "notes": [ { "step": 0, "len": 2, "pitch": "C4" } ] } }"#);
        assert!(d.validate().unwrap_err().contains("no such file"));
        let d = doc(r#"{ "name": "n", "duration": 0.5, "root": { "type": "seq",
                 "bpm": 120, "wave": "sampler",
                 "env": { "s": 1 },
                 "notes": [ { "step": 0, "len": 2, "pitch": "C4" } ] } }"#);
        assert!(d.validate().unwrap_err().contains("sf2"));
    }

    /// Full sampler audio check — needs a real SoundFont. Set
    /// TONO_TEST_SF2=/path/to/any_gm_bank.sf2 to enable; skipped (and
    /// printed as such) otherwise so CI stays hermetic.
    #[test]
    fn sampler_renders_real_instruments_deterministically() {
        let Some(sf2) = std::env::var_os("TONO_TEST_SF2") else {
            eprintln!("skipping sampler audio test: TONO_TEST_SF2 not set");
            return;
        };
        let sf2 = sf2.to_string_lossy().replace('"', "");
        let d = doc(&format!(
            r#"{{ "name": "n", "duration": 2.0, "root": {{ "type": "seq",
                 "bpm": 120, "wave": "sampler", "sf2": "{sf2}", "sf2_preset": 0,
                 "env": {{ "s": 1 }},
                 "notes": [ {{ "step": 0, "len": 2, "pitch": "C4" }},
                            {{ "step": 2, "len": 2, "pitch": "E4" }},
                            {{ "step": 4, "len": 4, "pitch": "G4" }} ] }} }}"#
        ));
        let s = render(&d);
        assert!(rms(&s) > 0.01, "sampled piano audible");
        assert_eq!(s, render(&d), "sampler render is deterministic");
        // Percussion bank: a GM kick on channel 9.
        let k = doc(&format!(
            r#"{{ "name": "n", "duration": 1.0, "root": {{ "type": "seq",
                 "bpm": 120, "wave": "sampler", "sf2": "{sf2}", "sf2_bank": 128,
                 "env": {{ "s": 1 }},
                 "notes": [ {{ "step": 0, "len": 2, "pitch": "midi:36" }} ] }} }}"#
        ));
        assert!(rms(&render(&k)[..8820]) > 0.01, "sampled kick audible");
    }

    #[test]
    fn duck_pumps_a_pad_under_its_trigger() {
        // A steady pad ducked by a kick pattern: rms right after each kick is
        // lower than between kicks.
        let d = doc(
            r#"{ "name": "n", "duration": 1.0, "root": { "type": "chain", "stages": [
                { "type": "sine", "freq": 220 },
                { "type": "duck", "amount": 0.9, "release": 0.2,
                  "trigger": { "type": "seq", "bpm": 120, "steps_per_beat": 1,
                    "wave": "kit", "env": { "s": 1 },
                    "notes": [ { "step": 0, "len": 1, "pitch": "midi:36" },
                               { "step": 1, "len": 1, "pitch": "midi:36" } ] } }
            ] } }"#,
        );
        let s = render(&d);
        let sr = 44_100;
        // 60 ms right after the kick at t=0 vs the recovered region ~0.4 s.
        let after_kick = rms(&s[..sr * 6 / 100]);
        let recovered = rms(&s[sr * 2 / 5..sr * 45 / 100]);
        assert!(
            after_kick < recovered * 0.65,
            "pumped {after_kick} vs recovered {recovered}"
        );
    }

    #[test]
    fn swing_delays_offbeats_and_humanize_jitters_deterministically() {
        let beat = |extra: &str| {
            let d = doc(&format!(
                r#"{{ "name": "n", "duration": 1.0, "root": {{ "type": "seq",
                     "bpm": 120, "steps_per_beat": 2, "wave": "sine"{extra},
                     "env": {{ "d": 0.05 }},
                     "notes": [ {{ "step": 0, "len": 1, "pitch": 880 }},
                                {{ "step": 1, "len": 1, "pitch": 880 }} ] }} }}"#
            ));
            render(&d)
        };
        let onset =
            |s: &[f32], from: usize| from + s[from..].iter().position(|x| x.abs() > 0.05).unwrap();
        let straight = beat("");
        let swung = beat(r#", "swing": 0.6"#);
        // Step 1 (the off-beat, at 0.25 s) lands later when swung; step 0 doesn't.
        let half = 44_100 / 5; // search after 0.2 s
        assert_eq!(onset(&straight, 0), onset(&swung, 0));
        let (a, b) = (onset(&straight, half), onset(&swung, half));
        let expected = (0.6 * 0.5 * 0.25 * 44_100.0) as usize; // swing*half*step
        assert!(
            (b - a) as i64 - expected as i64 <= 2,
            "off-beat delayed by ~{expected}, got {}",
            b - a
        );
        // Humanize changes timing/level but is deterministic.
        let h1 = beat(r#", "humanize": 0.3"#);
        let h2 = beat(r#", "humanize": 0.3"#);
        assert_eq!(h1, h2);
        assert_ne!(h1, straight);
    }

    #[test]
    fn cowbell_knocks_and_tracks_pitch() {
        let lo = one_note("cowbell", "A4", 1.0);
        let hi = one_note("cowbell", "A5", 1.0);
        assert!(rms(&lo[..4410]) > 0.1, "cowbell knocks");
        assert!(brightness(&hi) > brightness(&lo), "pitch tracks the note");
        // Fast knock decay: the tail is near-silent.
        assert!(rms(&lo[lo.len() / 2..]) < 0.01);
        // And the kit's fixed cowbell (GM 56) responds too.
        let kit = one_note("kit", "midi:56", 0.3);
        assert!(rms(&kit[..4410]) > 0.05, "kit cowbell audible");
    }

    #[test]
    fn kit_maps_pitches_to_distinct_drums() {
        let kick = one_note("kit", "midi:36", 0.4);
        let snare = one_note("kit", "midi:38", 0.4);
        let hat = one_note("kit", "midi:42", 0.4);
        for (name, s) in [("kick", &kick), ("snare", &snare), ("hat", &hat)] {
            assert!(rms(s) > 0.01, "{name} audible");
        }
        // Spectral ordering: kick < snare < hat.
        assert!(brightness(&kick) < brightness(&snare));
        assert!(brightness(&snare) < brightness(&hat));
        // Hat dies fast; open hat (midi:46) rings longer.
        let open = one_note("kit", "midi:46", 0.4);
        let q = hat.len() / 4;
        assert!(rms(&open[q..2 * q]) > rms(&hat[q..2 * q]) * 2.0);
        // Noise-based drums stay deterministic.
        assert_eq!(snare, one_note("kit", "midi:38", 0.4));
    }

    #[test]
    fn seq_with_absurd_note_lengths_stays_bounded() {
        // A 4-billion-step note and a near-zero bpm must not allocate
        // note-length buffers beyond the render window (OOM guard).
        let d = doc(r#"{ "name": "n", "duration": 0.1, "root": { "type": "seq",
                 "bpm": 120, "wave": "square", "env": { "d": 0.05 },
                 "notes": [ { "step": 0, "len": 4000000000, "pitch": 440 } ] } }"#);
        assert_eq!(render(&d).len(), 4410);
        let d = doc(r#"{ "name": "n", "duration": 0.1, "root": { "type": "seq",
                 "bpm": 0.0001, "wave": "sine", "env": { "d": 0.05 },
                 "notes": [ { "step": 0, "len": 1, "pitch": 440 } ] } }"#);
        assert_eq!(render(&d).len(), 4410);
    }

    #[test]
    fn mix_layers_and_mul_gates() {
        let d = doc(
            r#"{ "name": "n", "duration": 0.05, "root": { "type": "mix", "inputs": [
                { "type": "sine", "freq": 220 },
                { "type": "sine", "freq": 330 }
            ] } }"#,
        );
        assert!(rms(&render(&d)) > 0.5);
        let d = doc(
            r#"{ "name": "n", "duration": 0.05, "root": { "type": "mul", "inputs": [
                { "type": "sine", "freq": 220 },
                { "type": "gain", "amount": 1 }
            ] } }"#,
        );
        // gain (a processor) standalone renders silence; mul with silence = silence.
        assert!(rms(&render(&d)) < 1e-6);
    }
}
