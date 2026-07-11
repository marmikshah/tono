//! The wait-free SPSC transport: a lock-free sample ring joining a control
//! thread ([`Pump`]) to the audio callback ([`Renderer`]).

use std::sync::Arc;
use std::sync::atomic::{AtomicU32, AtomicUsize, Ordering};

use super::engine::Engine;
use super::source::AudioSource;

/// slot is an `AtomicU32` (the sample's bits), so it is entirely safe — no
/// `unsafe`, no locks. The [`Controller`] is the sole producer, the [`Renderer`]
/// the sole consumer.
pub(super) struct SampleRing {
    buf: Vec<AtomicU32>,
    /// Next slot the consumer reads.
    head: AtomicUsize,
    /// Next slot the producer writes.
    tail: AtomicUsize,
}

impl SampleRing {
    pub(super) fn new(capacity: usize) -> Self {
        // One slot stays empty so full and empty are distinguishable.
        let n = (capacity + 1).max(2);
        SampleRing {
            buf: (0..n).map(|_| AtomicU32::new(0)).collect(),
            head: AtomicUsize::new(0),
            tail: AtomicUsize::new(0),
        }
    }

    fn cap(&self) -> usize {
        self.buf.len()
    }

    pub(super) fn len(&self) -> usize {
        let t = self.tail.load(Ordering::Acquire);
        let h = self.head.load(Ordering::Acquire);
        (t + self.cap() - h) % self.cap()
    }

    fn free(&self) -> usize {
        self.cap() - 1 - self.len()
    }

    /// Push one sample (producer). Returns false if full.
    pub(super) fn push(&self, sample: f32) -> bool {
        let tail = self.tail.load(Ordering::Relaxed);
        let next = (tail + 1) % self.cap();
        if next == self.head.load(Ordering::Acquire) {
            return false;
        }
        self.buf[tail].store(sample.to_bits(), Ordering::Relaxed);
        self.tail.store(next, Ordering::Release);
        true
    }

    /// Pop one sample (consumer), or `None` if empty.
    pub(super) fn pop(&self) -> Option<f32> {
        let head = self.head.load(Ordering::Relaxed);
        if head == self.tail.load(Ordering::Acquire) {
            return None;
        }
        let bits = self.buf[head].load(Ordering::Relaxed);
        self.head.store((head + 1) % self.cap(), Ordering::Release);
        Some(f32::from_bits(bits))
    }
}

/// The control side of a split source: owns any [`AudioSource`] and produces
/// audio into the shared ring. Lives on any non-audio thread — deref to call
/// every method of the wrapped source (for an [`Engine`]: `play`, `set_gain`,
/// `set_param`, …), then [`pump`](Self::pump) to keep the audio thread fed.
///
/// Generic over the source so the same seam drives a bare [`Engine`], a
/// [`Mixer`] of instruments + SFX, or any other [`AudioSource`]. [`Controller`]
/// is the `Engine` specialization returned by [`Engine::split`].
pub struct Pump<S: AudioSource> {
    source: S,
    ring: Arc<SampleRing>,
    pump_buf: Vec<f32>,
}

impl<S: AudioSource> Pump<S> {
    /// Mix up to `frames` frames and hand them to the audio thread. Returns the
    /// number of frames actually pushed — fewer when the ring is full, which is
    /// the backpressure signal to stop pumping until the next tick.
    pub fn pump(&mut self, frames: usize) -> usize {
        // Render only what the ring can take. `fill` advances every play head,
        // so a frame rendered but not pushed would be audio lost forever — not
        // deferred. As the sole producer, the space observed here can only grow
        // before the pushes below.
        let frames = frames.min(self.ring.free() / 2);
        if frames == 0 {
            return 0;
        }
        if self.pump_buf.len() < frames * 2 {
            self.pump_buf.resize(frames * 2, 0.0);
        }
        let block = &mut self.pump_buf[..frames * 2];
        self.source.fill(block);
        for s in block.iter() {
            self.ring.push(*s);
        }
        frames
    }
}

impl<S: AudioSource> std::ops::Deref for Pump<S> {
    type Target = S;
    fn deref(&self) -> &S {
        &self.source
    }
}

impl<S: AudioSource> std::ops::DerefMut for Pump<S> {
    fn deref_mut(&mut self) -> &mut S {
        &mut self.source
    }
}

/// The control side of a split [`Engine`] — see [`Pump`] and [`Engine::split`].
pub type Controller = Pump<Engine>;

/// Split any [`AudioSource`] into a [`Pump`] (control thread) and a [`Renderer`]
/// (audio thread) joined by a wait-free ring `ring_frames` deep. Pump the
/// controller off the audio thread; the renderer drains it in the callback.
pub fn spsc<S: AudioSource>(source: S, ring_frames: usize) -> (Pump<S>, Renderer) {
    let ring = Arc::new(SampleRing::new(ring_frames * 2));
    (
        Pump {
            source,
            ring: ring.clone(),
            pump_buf: Vec::new(),
        },
        Renderer { ring },
    )
}

/// The audio side of a split engine: drains the ring in the output callback.
/// `Send`, alloc-free, lock-free — safe to hand to a cpal / AudioWorklet thread.
/// On underrun it writes silence (the ring depth chosen at [`Engine::split`]
/// trades latency against underrun safety).
pub struct Renderer {
    pub(super) ring: Arc<SampleRing>,
}

impl AudioSource for Renderer {
    fn fill(&mut self, out: &mut [f32]) -> usize {
        // Drain whole frames only: taking half a frame across an underrun
        // would land every later left sample on a right slot — a permanent
        // channel swap. As the sole consumer, an observed pair stays there.
        for frame in out.chunks_mut(2) {
            if frame.len() == 2 && self.ring.len() >= 2 {
                frame[0] = self.ring.pop().unwrap_or(0.0);
                frame[1] = self.ring.pop().unwrap_or(0.0);
            } else {
                frame.fill(0.0);
            }
        }
        out.len() / 2
    }
}
