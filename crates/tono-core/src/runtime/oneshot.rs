//! A pre-rendered one-shot — a stinger — mixed into a stereo block without
//! touching the allocator: the render happened at schedule time, the fire
//! only reads a buffer.

use std::sync::Arc;

use super::ramp::{Ramp, Tween};

/// A sounding one-shot: an interleaved stereo buffer, its play head and a
/// declick gain ramp.
pub(crate) struct OneShot {
    buf: Arc<[f32]>,
    /// Play head in samples (frames × 2).
    pos: usize,
    gain: Ramp,
}

impl OneShot {
    /// A one-shot over `buf` that ramps from unity to `gain` over `declick`
    /// (`Tween::IMMEDIATE` plays it as rendered).
    pub(crate) fn new(buf: Arc<[f32]>, gain: f32, declick: Tween) -> Self {
        let mut ramp = Ramp::new(1.0);
        ramp.set(gain.max(0.0), declick);
        OneShot {
            buf,
            pos: 0,
            gain: ramp,
        }
    }

    /// Interleave planar channels into a one-shot at unity gain. Channels of
    /// unequal length are truncated to the shorter one, so the mix never
    /// indexes past either.
    pub(crate) fn from_planar(left: &[f32], right: &[f32]) -> Self {
        let n = left.len().min(right.len());
        let mut buf = Vec::with_capacity(n * 2);
        for i in 0..n {
            buf.push(left[i]);
            buf.push(right[i]);
        }
        OneShot::new(buf.into(), 1.0, Tween::IMMEDIATE)
    }

    /// Add the next frames to `out` (interleaved stereo), advancing the head.
    pub(crate) fn mix_into(&mut self, out: &mut [f32]) {
        let frames = out.len() / 2;
        let take = ((self.buf.len() - self.pos) / 2).min(frames);
        for f in 0..take {
            let g = self.gain.tick();
            out[f * 2] += self.buf[self.pos + f * 2] * g;
            out[f * 2 + 1] += self.buf[self.pos + f * 2 + 1] * g;
        }
        self.pos += take * 2;
    }

    /// Whether every sample has been mixed out.
    pub(crate) fn done(&self) -> bool {
        self.pos >= self.buf.len()
    }
}
