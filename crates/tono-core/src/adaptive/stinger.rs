//! A stinger rendered ahead of time: the render is the slow part, so a caller
//! does it on any thread and hands the result to an [`AdaptiveMusic`] later.

use std::sync::Arc;

use super::{Action, AdaptiveMusic, Quantize, SoundDoc};
use crate::runtime::Tween;
use crate::runtime::oneshot::OneShot;

/// Interleaved stereo audio for one stinger, ready to fire. Cloning shares
/// the buffer, so one render can play any number of times.
///
/// ```
/// use tono_core::adaptive::{AdaptiveMusic, Quantize, Stinger};
/// use tono_core::dsl::{Node, SoundDoc};
///
/// let hit = SoundDoc::new("hit", Node::Sine { freq: 660.0.into() });
/// // Render on a worker; only the hand-in touches the music bed.
/// let stinger = std::thread::spawn(move || Stinger::render(&hit, 48_000))
///     .join()
///     .unwrap();
///
/// let mut music = AdaptiveMusic::new(48_000);
/// music.set_tempo(104.0, 4);
/// music.play_stinger_at(&stinger, Quantize::AtBeat(8)); // lands on beat 8
/// ```
#[derive(Clone, Debug)]
pub struct Stinger {
    /// `[L, R, L, R, …]`.
    buf: Arc<[f32]>,
}

impl Stinger {
    /// Render `doc` at `sample_rate`, the rate of the [`AdaptiveMusic`] that
    /// will play it. A mono document plays on both channels. The same render
    /// [`AdaptiveMusic::stinger`] does, so the result is byte-identical.
    pub fn render(doc: &SoundDoc, sample_rate: u32) -> Self {
        let (left, right) =
            crate::render::render_product(&doc.at_sample_rate(sample_rate)).into_stereo();
        Stinger::from_stereo(&left, &right)
    }

    /// Build a stinger from planar stereo already rendered at the music's
    /// rate. Channels of unequal length are cut to the shorter one.
    pub fn from_stereo(left: &[f32], right: &[f32]) -> Self {
        let n = left.len().min(right.len());
        let mut buf = Vec::with_capacity(n * 2);
        for i in 0..n {
            buf.push(left[i]);
            buf.push(right[i]);
        }
        Stinger { buf: buf.into() }
    }

    /// Length in frames.
    pub fn frames(&self) -> usize {
        self.buf.len() / 2
    }

    /// A sounding one-shot over the shared buffer, at unity gain.
    pub(super) fn one_shot(&self) -> OneShot {
        OneShot::new(Arc::clone(&self.buf), 1.0, Tween::IMMEDIATE)
    }

    #[cfg(test)]
    pub(super) fn shares_buffer_with(&self, other: &Stinger) -> bool {
        Arc::ptr_eq(&self.buf, &other.buf)
    }
}

impl AdaptiveMusic {
    /// Render a stinger at this bed's rate. Same as [`Stinger::render`]; call
    /// that one from a thread that cannot reach the bed.
    pub fn render_stinger(&self, doc: &SoundDoc) -> Stinger {
        Stinger::render(doc, self.sample_rate)
    }

    /// Fire a pre-rendered stinger over the bed now. Does not render or copy
    /// the audio.
    pub fn play_stinger(&mut self, stinger: &Stinger) {
        self.reserve_stinger_slot();
        self.stingers.push(stinger.one_shot());
    }

    /// Fire a pre-rendered stinger on a beat/bar boundary. With
    /// [`Quantize::AtBeat`] the caller names the beat, so the time spent
    /// rendering cannot push it onto a later one.
    pub fn play_stinger_at(&mut self, stinger: &Stinger, q: Quantize) {
        self.reserve_stinger_slot();
        self.apply_or_schedule(q, Action::Stinger(stinger.one_shot()));
    }

    /// Keep room in `stingers` for every pending stinger plus one more. A
    /// scheduled stinger is pushed by `fire_due` inside `fill`, which must not
    /// allocate, so the growth happens here on the caller's thread instead.
    fn reserve_stinger_slot(&mut self) {
        let pending = self
            .pending
            .iter()
            .filter(|e| matches!(e.action, Action::Stinger(_)))
            .count();
        self.stingers.reserve(pending + 1);
    }
}
