//! The crate's output seam: the [`AudioSource`] trait every adapter targets,
//! the interleaved-stereo channel spread, and the allocation-free
//! [`StreamSource`] over a streamable doc.

use crate::dsl::SoundDoc;
use crate::streaming::StreamGraph;

/// A block-serving audio source: fill `out` (interleaved stereo L,R,L,R…) and
/// return the number of frames written. Runs indefinitely. This is the single
/// seam host output adapters target, so a `cpal` callback, an AudioWorklet, or a
/// Bevy source never depend on a concrete engine type.
///
/// Implementations overwrite the **whole** `out` buffer (silence where there is
/// nothing to play), so a caller may mix several sources through one scratch
/// buffer without re-zeroing.
pub trait AudioSource {
    /// Fill `out` with the next block of interleaved-stereo audio.
    fn fill(&mut self, out: &mut [f32]) -> usize;

    /// Rewind the source to its start (playback position / phase to zero).
    /// Defaults to a no-op; a looping source overrides it so a transport can
    /// restart it from the top. [`AdaptiveMusic::reset`](crate::adaptive::AdaptiveMusic::reset)
    /// calls this on each layer.
    fn reset(&mut self) {}
}

/// Spread an interleaved-stereo buffer across a device's channel layout: mono
/// devices get the mid (`0.5 * (l + r)`), stereo gets L/R, extra channels are
/// zeroed. The one channel-adaptation every output adapter (cpal callback,
/// AudioWorklet shim) needs — pure sample shuffling, no device dependency.
/// `data` holds `channels` interleaved device channels; `stereo` holds the same
/// frame count as L,R pairs.
pub fn write_interleaved(data: &mut [f32], channels: usize, stereo: &[f32]) {
    let channels = channels.max(1);
    // Never read past the source: a caller handing a short `stereo` slice must
    // not panic on the audio thread — fill only the frames we actually have.
    let frames = (data.len() / channels).min(stereo.len() / 2);
    for f in 0..frames {
        let (l, r) = (stereo[f * 2], stereo[f * 2 + 1]);
        let base = f * channels;
        if channels == 1 {
            data[base] = 0.5 * (l + r);
        } else {
            data[base] = l;
            data[base + 1] = r;
            for c in 2..channels {
                data[base + c] = 0.0;
            }
        }
    }
}
/// doc's graph **indefinitely and allocation-free** — mono duplicated to
/// stereo. Returns `None` for docs outside the streamable subset (the caller
/// falls back to a buffer-backed [`Player`]/instance). This is how a game
/// feeds the streaming renderer straight to a cpal / AudioWorklet callback
/// for continuous generative content.
///
/// Byte-identity: the offline bounce ends in a transparent sample-peak safety
/// limit, a whole-buffer gain that cannot be computed causally. [`from_doc`]
/// (`StreamSource::from_doc`) therefore measures the finite render's peak with
/// one throwaway pass of the same deterministic graph (O(duration) time, O(1)
/// memory) and bakes the identical constant gain, so the stream matches the
/// bounce bit-for-bit over the document's duration.
pub struct StreamSource {
    graph: StreamGraph,
    scratch: Vec<f32>,
    /// The bounce's peak-limit gain (1.0 when the doc never exceeds the ceiling).
    gain: f32,
}

impl StreamSource {
    /// Build a streaming source for `doc`, or `None` if it isn't streamable.
    pub fn from_doc(doc: &SoundDoc) -> Option<Self> {
        let graph = StreamGraph::try_from_doc(doc)?;
        // Probe pass: same graph, same bytes — find the peak the offline
        // output stage would have limited against.
        let mut probe = StreamGraph::try_from_doc(doc)?;
        let mut remaining = ((doc.duration * doc.sample_rate as f32).ceil() as usize).max(1);
        let mut block = [0.0f32; 1024];
        let mut peak = 0.0f32;
        while remaining > 0 {
            let take = block.len().min(remaining);
            probe.fill(&mut block[..take]);
            peak = block[..take].iter().fold(peak, |m, x| m.max(x.abs()));
            remaining -= take;
        }
        let gain = if peak > crate::dsp::CEIL {
            crate::dsp::CEIL / peak
        } else {
            1.0
        };
        Some(StreamSource {
            graph,
            scratch: Vec::new(),
            gain,
        })
    }
}

impl AudioSource for StreamSource {
    fn fill(&mut self, out: &mut [f32]) -> usize {
        let frames = out.len() / 2;
        if self.scratch.len() < frames {
            self.scratch.resize(frames, 0.0);
        }
        let mono = &mut self.scratch[..frames];
        self.graph.fill(mono);
        for f in 0..frames {
            let v = mono[f] * self.gain;
            out[f * 2] = v;
            out[f * 2 + 1] = v;
        }
        frames
    }
}
