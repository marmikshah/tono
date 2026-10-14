//! The `tracks` bounce: the mixer evaluated in one block (see
//! [`crate::streaming`]), then the whole-buffer output stage. Also the pan
//! law, the per-track RNG streams and the automation-lane cursor the mixer
//! uses.

use super::Signal;
use super::output::{make_loop_buffer, normalize_output};
#[cfg(feature = "sampler")]
use super::seq::{SeqVoice, sampler_seq_stereo};
#[cfg(feature = "sampler")]
use crate::dsl::SeqWave;
use crate::dsl::{AutoCurve, AutoLane, AutoPoint, AutoTarget, Node, Playback, SoundDoc};
use crate::dsp::peak_limit;

/// Equal-power pan gains for a track's `(pan, gain)`.
pub(crate) fn pan_gains(pan: f32, gain: f32) -> (f32, f32) {
    let theta = (pan + 1.0) * std::f32::consts::FRAC_PI_4;
    (crate::dsp::cos(theta) * gain, crate::dsp::sin(theta) * gain)
}

/// Derive a track's independent RNG stream from the document seed.
/// `stream` is the track's FNV stream key, not a track index. SplitMix64
/// finalizer over a golden-gamma offset.
pub(crate) fn track_stream_seed(seed: u64, stream: u64) -> u64 {
    crate::dsp::splitmix_mix(
        seed ^ stream
            .wrapping_add(1)
            .wrapping_mul(crate::dsp::GOLDEN_GAMMA),
    )
}

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

/// A persistent cursor over one automation lane. [`LaneCursor::at`] must be
/// called with monotonically non-decreasing sample indices (the segment
/// cursor only advances); starting mid-lane is fine — the strict-`>` advance
/// picks the same segment a from-zero scan would.
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
    /// exact segment a from-zero scan picks — a sample landing on a
    /// breakpoint interpolates in the earlier segment.
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

/// Render the stereo mixing console, applying tracks, buses and master inserts.
/// Each named track owns an independent deterministic RNG stream; editing or
/// muting one track preserves its siblings. Loop and normalization apply jointly.
pub fn render_tracks(doc: &SoundDoc) -> Option<TracksRender> {
    bounce(doc, false).map(|(r, _)| r)
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
    let (_, stems) = bounce(doc, true)?;
    Some(stems.expect("stems requested"))
}

/// The mixer in one block, then the output stage: loop body, optional
/// normalization, and the joint sample-peak safety limit.
fn bounce(doc: &SoundDoc, want_stems: bool) -> Option<(TracksRender, Option<Vec<Stem>>)> {
    let mix = crate::streaming::mix_offline(doc, want_stems)?;
    let sr = doc.sample_rate;
    let (mut left, mut right) = (mix.left, mix.right);
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
            layers: mix.layers,
        },
        mix.stems,
    ))
}

/// A track whose node is directly a sampler seq renders in native stereo.
#[cfg(feature = "sampler")]
pub(crate) fn track_native_stereo(node: &Node, n: usize, sr: u32) -> Option<(Signal, Signal)> {
    let (voice, bpm, steps_per_beat, notes) = SeqVoice::from_node(node)?;
    if voice.wave != SeqWave::Sampler {
        return None;
    }
    let step_dur = sr as f32 * 60.0 / bpm / steps_per_beat.max(1) as f32;
    sampler_seq_stereo(&voice, notes, step_dur, n, sr)
}

/// Without the `sampler` feature there is no native-stereo SoundFont path.
#[cfg(not(feature = "sampler"))]
pub(crate) fn track_native_stereo(_node: &Node, _n: usize, _sr: u32) -> Option<(Signal, Signal)> {
    None
}
