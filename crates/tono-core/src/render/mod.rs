//! The bounce: a document rendered to samples by the one block evaluator in
//! [`crate::streaming`], followed by the whole-buffer output stage (loop
//! body, normalization, peak limit, stereo treatment).
//!
//! Rendering is a pure function of `(graph, seed, sample_rate)`.

mod effects;
mod kit;
mod osc;
mod seq;
#[cfg(test)]
mod tests;

pub(crate) use effects::{
    ConvolveSpec, FilterKind, GranularSpec, biquad_coeffs, convolve, drive_antideriv, drive_curve,
    granular,
};
pub(crate) use osc::{osc, poly_blep, wavetable_frames, wavetable_lookup};
pub(crate) use seq::seq_to_signal;

use crate::dsl::{Adsr, Playback, SoundDoc, Value};
use crate::dsp::peak_limit;

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

impl RenderProduct {
    /// Consume the rendered bus, duplicating mono when there is no mixer.
    /// Output-stage Haas/Wide treatment remains the player's/exporter's job.
    pub fn into_stereo(self) -> (Vec<f32>, Vec<f32>) {
        self.stereo
            .unwrap_or_else(|| (self.mono.clone(), self.mono))
    }
}

mod output;
mod tracks;

use output::normalize_output;
pub use output::{loop_seam_db, make_loop_buffer, stereoize};
pub(crate) use tracks::{
    LaneCursor, MASTER_STREAM, pan_gains, track_native_stereo, track_stream_seed,
};
pub use tracks::{LayerStats, Stem, TracksRender, render_stems, render_tracks};

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

/// Raw graph evaluation — the root in one block with no output stage (loop
/// / normalize / peak-limit / stereo). The reference the streaming tests
/// compare chunked fills against.
#[cfg(test)]
pub(crate) fn render_graph(doc: &SoundDoc) -> Signal {
    crate::streaming::render_graph_offline(&doc.root, doc.sample_rate, doc.frame_count(), doc.seed)
}

/// The non-mixer render path: one graph, one mono buffer.
fn render_plain(doc: &SoundDoc) -> Signal {
    let sr = doc.sample_rate;
    let mut out =
        crate::streaming::render_graph_offline(&doc.root, sr, doc.frame_count(), doc.seed);
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
        Some(nz) => normalize_output(&mut [&mut out], nz, sr),
        // Default: a transparent sample-peak safety limit only.
        None => peak_limit(&mut [&mut out]),
    }
    out
}

/// Evaluate a parameter over `n` samples with the shared per-sample
/// evaluator — the seq synthesis pre-renders its per-note pitch and duty
/// buffers this way.
pub(super) fn eval_value(v: &Value, n: usize, sr: u32) -> Vec<f32> {
    let mut val = crate::streaming::value::Val::build(v, sr, n);
    (0..n).map(|t| val.eval(t)).collect()
}

/// Edit-stable seed for a [`Modulator::Rand`](crate::dsl::Modulator::Rand): a
/// hash of only the modulator's own fields, so its walk never shifts when
/// sibling nodes are added or removed (the random stream is not threaded
/// through graph traversal).
pub(crate) fn rand_seed(seed: u64, from: f32, to: f32, rate: f32) -> u64 {
    let mut h = seed ^ crate::dsp::GOLDEN_GAMMA;
    for bits in [from.to_bits(), to.to_bits(), rate.to_bits()] {
        h = (h ^ bits as u64).wrapping_mul(crate::dsp::FNV_PRIME);
    }
    h
}

/// ADSR envelope with an sfxr-style punch boost on the initial transient,
/// over `n` samples — the seq synthesis's per-note amplitude envelope.
pub(super) fn adsr(env: &Adsr, n: usize, sr: u32) -> Signal {
    let Adsr { a, d, s, r, punch } = *env;
    let srf = sr as f32;
    let rel_start = (n as f32 / srf - r).max(0.0);
    (0..n)
        .map(|i| crate::dsp::adsr_env(i as f32 / srf, a, d, s, r, punch, rel_start))
        .collect()
}
