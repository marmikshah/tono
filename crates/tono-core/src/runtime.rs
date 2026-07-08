//! runtime — the embeddable real-time control surface over the deterministic engine.
//!
//! The idiomatic library API a game (or any host) drives: [`Engine::load`] a
//! [`SoundDoc`] (or [`Engine::load_patch`] a [`Patch`] with named parameters) as
//! a reusable **resource**, [`Engine::play`] as many independent **instances** as
//! you like, and control each by its [`InstanceHandle`] with [`Tween`]-smoothed
//! setters. Host output adapters (cpal, an AudioWorklet, a Bevy source) target
//! the [`AudioSource`] trait, so they never depend on a concrete engine type.
//!
//! Backed today by the deterministic buffer renderer ([`crate::player::Player`]),
//! which keeps the mix **byte-identical to an offline bounce**. Instance master
//! controls (gain / pan / stop) apply live per block; parameter and layer-gain
//! changes ([`Engine::set_param`] / [`Engine::set_layer_gain`]) re-render the
//! instance and **crossfade** for a click-free swap — control-rate today, and
//! sample-accurate once the stateful streaming renderer lands behind this same
//! seam. Multi-threaded real-time use goes through [`Engine::split`].
//!
//! # Adapters
//!
//! A host output is a thin shim over [`AudioSource`] + [`Engine::split`]. cpal:
//!
//! ```ignore
//! let (mut control, mut audio) = Engine::new(sr).split(2048);
//! let stream = device.build_output_stream(
//!     &config,
//!     move |out: &mut [f32], _| { audio.fill(out); }, // audio thread drains the ring
//!     err_fn, None,
//! )?;
//! stream.play()?;
//! // On a control thread: loop { control.pump(1024); std::thread::sleep(dt); }
//! ```
//!
//! A Bevy `Decodable` / rodio `Source` wraps the same [`Renderer`]; an
//! AudioWorklet calls [`AudioSource::fill`] on each 128-frame quantum.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, AtomicUsize, Ordering};

use crate::dsl::{Node, SoundDoc};
use crate::edit::{EditOp, apply_ops};
use crate::patch::Patch;
use crate::player::Player;
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

/// Handle to a loaded patch — an immutable, shareable resource. Cheap to copy;
/// spawn as many instances of it as you like.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct PatchId(usize);

/// Handle to one live instance of a patch. Stable for the instance's lifetime;
/// setters against a finished/unknown handle are no-ops.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct InstanceHandle(u64);

/// A named patch parameter resolved to a typed handle at load — poke it by id on
/// the hot path, never by string. Scoped to the [`PatchId`] it came from.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct ParamId {
    patch: usize,
    index: usize,
}

/// A named mixer layer (a `tracks` entry) resolved to a typed handle at load.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct LayerId {
    patch: usize,
    index: usize,
}

/// Control-rate linear smoothing over a fixed duration. `Tween::default()` /
/// [`Tween::IMMEDIATE`] jumps instantly; [`Tween::ms`] ramps over a wall-clock time.
#[derive(Clone, Copy, Debug)]
pub struct Tween {
    frames: u32,
}

impl Tween {
    /// Jump to the target immediately (no ramp).
    pub const IMMEDIATE: Tween = Tween { frames: 0 };

    /// Ramp over an explicit number of frames.
    pub const fn frames(n: u32) -> Self {
        Tween { frames: n }
    }

    /// Ramp over `ms` milliseconds at `sample_rate`.
    pub fn ms(ms: f32, sample_rate: u32) -> Self {
        let f = (ms / 1000.0 * sample_rate as f32).round();
        Tween {
            frames: if f > 0.0 { f as u32 } else { 0 },
        }
    }
}

impl Default for Tween {
    fn default() -> Self {
        Tween::IMMEDIATE
    }
}

/// A per-frame linear ramp toward a target value.
#[derive(Clone, Copy)]
struct Ramp {
    value: f32,
    target: f32,
    step: f32,
    remaining: u32,
}

impl Ramp {
    fn new(v: f32) -> Self {
        Ramp {
            value: v,
            target: v,
            step: 0.0,
            remaining: 0,
        }
    }

    fn set(&mut self, target: f32, tw: Tween) {
        self.target = target;
        if tw.frames == 0 {
            self.value = target;
            self.step = 0.0;
            self.remaining = 0;
        } else {
            self.step = (target - self.value) / tw.frames as f32;
            self.remaining = tw.frames;
        }
    }

    /// Advance one frame and return the current value.
    fn tick(&mut self) -> f32 {
        if self.remaining > 0 {
            self.value += self.step;
            self.remaining -= 1;
            if self.remaining == 0 {
                self.value = self.target;
            }
        }
        self.value
    }

    fn at_target(&self) -> bool {
        self.remaining == 0
    }
}

/// Equal-level stereo balance (unity at centre): `pan` −1 = hard left, +1 = hard
/// right. Returns per-channel gains `(left, right)`.
fn balance(pan: f32) -> (f32, f32) {
    let l = if pan <= 0.0 { 1.0 } else { 1.0 - pan };
    let r = if pan >= 0.0 { 1.0 } else { 1.0 + pan };
    (l, r)
}

/// The tracks (layers) of a document, in order, by optional id.
fn layer_ids(doc: &SoundDoc) -> Vec<Option<String>> {
    match &doc.root {
        Node::Tracks { tracks, .. } => tracks.iter().map(|t| t.id.clone()).collect(),
        _ => Vec::new(),
    }
}

struct Instance {
    id: u64,
    patch: usize,
    /// Current parameter values (name → value); seeds the next re-render.
    values: BTreeMap<String, f32>,
    /// Per-layer gain overrides (track index → gain).
    layer_gains: BTreeMap<usize, f32>,
    player: Player,
    /// The outgoing player during a click-free re-render crossfade, with the
    /// incoming mix ramp (0 → 1).
    fading_in: Option<(Player, Ramp)>,
    gain: Ramp,
    pan: Ramp,
    /// Fading out to be culled once silent.
    stopping: bool,
}

/// The runtime mixer: owns patch resources and their live instances, and serves
/// their mixed-down stereo through [`AudioSource::fill`].
pub struct Engine {
    sample_rate: u32,
    patches: Vec<Patch>,
    instances: Vec<Instance>,
    next_id: u64,
    buf_a: Vec<f32>,
    buf_b: Vec<f32>,
}

impl Engine {
    /// A fresh engine that renders at `sample_rate`.
    pub fn new(sample_rate: u32) -> Self {
        Engine {
            sample_rate,
            patches: Vec::new(),
            instances: Vec::new(),
            next_id: 1,
            buf_a: Vec::new(),
            buf_b: Vec::new(),
        }
    }

    /// The engine's sample rate.
    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    /// Load a bare document as a patch resource (no named parameters).
    pub fn load(&mut self, doc: &SoundDoc) -> PatchId {
        self.load_patch(&Patch {
            doc: doc.clone(),
            params: Vec::new(),
        })
    }

    /// Load a parametric [`Patch`] as a resource; its named params become
    /// [`ParamId`]s via [`Engine::param`].
    pub fn load_patch(&mut self, patch: &Patch) -> PatchId {
        self.patches.push(patch.clone());
        PatchId(self.patches.len() - 1)
    }

    /// Resolve a named parameter of a patch to a typed handle (once, off the hot
    /// path). `None` if the patch has no such param.
    pub fn param(&self, patch: PatchId, name: &str) -> Option<ParamId> {
        self.patches[patch.0]
            .params
            .iter()
            .position(|p| p.name == name)
            .map(|index| ParamId {
                patch: patch.0,
                index,
            })
    }

    /// Resolve a named mixer layer (a `tracks` entry) to a typed handle. `None`
    /// if the patch's graph has no track with that id.
    pub fn layer(&self, patch: PatchId, name: &str) -> Option<LayerId> {
        layer_ids(&self.patches[patch.0].doc)
            .iter()
            .position(|id| id.as_deref() == Some(name))
            .map(|index| LayerId {
                patch: patch.0,
                index,
            })
    }

    /// Spawn a one-shot instance of a patch (plays once, then culls itself).
    pub fn play(&mut self, patch: PatchId) -> InstanceHandle {
        self.spawn(patch, false)
    }

    /// Spawn a looping instance of a patch (plays until [`stop`](Self::stop)ped).
    pub fn play_looping(&mut self, patch: PatchId) -> InstanceHandle {
        self.spawn(patch, true)
    }

    fn spawn(&mut self, patch: PatchId, looping: bool) -> InstanceHandle {
        let values = self.patches[patch.0].defaults();
        let doc = self.build_doc(patch.0, &values, &BTreeMap::new());
        let mut player = self.new_player(doc, looping, 0);
        player.play();
        let id = self.next_id;
        self.next_id += 1;
        self.instances.push(Instance {
            id,
            patch: patch.0,
            values,
            layer_gains: BTreeMap::new(),
            player,
            fading_in: None,
            gain: Ramp::new(1.0),
            pan: Ramp::new(0.0),
            stopping: false,
        });
        InstanceHandle(id)
    }

    /// Instantiate the patch with `values` and apply per-layer gain overrides;
    /// falls back to the bare template if an edit fails (never a corrupt graph).
    fn build_doc(
        &self,
        patch: usize,
        values: &BTreeMap<String, f32>,
        layer_gains: &BTreeMap<usize, f32>,
    ) -> SoundDoc {
        let p = &self.patches[patch];
        let doc = p.instantiate(values).unwrap_or_else(|_| p.doc.clone());
        if layer_gains.is_empty() {
            return doc;
        }
        let ops: Vec<EditOp> = layer_gains
            .iter()
            .map(|(i, g)| EditOp::Set {
                path: format!("root.tracks[{i}].gain"),
                value: serde_json::json!(g),
            })
            .collect();
        apply_ops(&doc, &ops).unwrap_or(doc)
    }

    fn new_player(&self, mut doc: SoundDoc, looping: bool, seek: usize) -> Player {
        // Instances render at the engine's rate, not each doc's own — one internal
        // rate for the whole mix (resampling to the device belongs at the adapter).
        doc.sample_rate = self.sample_rate;
        let mut player = Player::new(doc);
        player.looping = looping;
        player.seek(seek);
        player.play();
        player
    }

    fn instance_mut(&mut self, h: InstanceHandle) -> Option<&mut Instance> {
        self.instances.iter_mut().find(|i| i.id == h.0)
    }

    fn find(&self, h: InstanceHandle) -> Option<usize> {
        self.instances.iter().position(|i| i.id == h.0)
    }

    /// Set an instance's linear gain, smoothed over `tw` (no-op if finished).
    pub fn set_gain(&mut self, h: InstanceHandle, gain: f32, tw: Tween) {
        if let Some(i) = self.instance_mut(h) {
            i.gain.set(gain.max(0.0), tw);
        }
    }

    /// Set an instance's stereo balance (−1 left … +1 right), smoothed over `tw`.
    pub fn set_pan(&mut self, h: InstanceHandle, pan: f32, tw: Tween) {
        if let Some(i) = self.instance_mut(h) {
            i.pan.set(pan.clamp(-1.0, 1.0), tw);
        }
    }

    /// Set a named parameter on a live instance, crossfading over `tw` for a
    /// click-free swap. `param` must come from the same patch the instance plays.
    pub fn set_param(&mut self, h: InstanceHandle, param: ParamId, value: f32, tw: Tween) {
        let Some(idx) = self.find(h) else { return };
        if self.instances[idx].patch != param.patch {
            return;
        }
        let name = self.patches[param.patch].params[param.index].name.clone();
        self.instances[idx].values.insert(name, value);
        self.rerender(idx, tw);
    }

    /// Set a named layer's gain on a live instance, crossfading over `tw`.
    pub fn set_layer_gain(&mut self, h: InstanceHandle, layer: LayerId, gain: f32, tw: Tween) {
        let Some(idx) = self.find(h) else { return };
        if self.instances[idx].patch != layer.patch {
            return;
        }
        self.instances[idx].layer_gains.insert(layer.index, gain);
        self.rerender(idx, tw);
    }

    /// Re-render instance `idx` from its current values and crossfade the outgoing
    /// audio into the new render over `tw` (a min fade avoids clicks).
    fn rerender(&mut self, idx: usize, tw: Tween) {
        let (patch, looping, pos) = {
            let i = &self.instances[idx];
            (i.patch, i.player.looping, i.player.position())
        };
        let values = self.instances[idx].values.clone();
        let layer_gains = self.instances[idx].layer_gains.clone();
        let doc = self.build_doc(patch, &values, &layer_gains);
        let fresh = self.new_player(doc, looping, pos);

        let fade = if tw.frames == 0 {
            Tween::ms(8.0, self.sample_rate)
        } else {
            tw
        };
        let inst = &mut self.instances[idx];
        // The current player becomes the outgoing one; the fresh render fades in.
        let outgoing = std::mem::replace(&mut inst.player, fresh);
        let mut mix = Ramp::new(0.0);
        mix.set(1.0, fade);
        inst.fading_in = Some((outgoing, mix));
    }

    /// Stop an instance with a declick fade-out; it is culled once silent. A
    /// zero-length `fade` is bumped to a short default so stops never click.
    pub fn stop(&mut self, h: InstanceHandle, fade: Tween) {
        let min_fade = Tween::ms(5.0, self.sample_rate);
        if let Some(i) = self.instance_mut(h) {
            let fade = if fade.frames == 0 { min_fade } else { fade };
            i.gain.set(0.0, fade);
            i.stopping = true;
        }
    }

    /// Number of live instances.
    pub fn active(&self) -> usize {
        self.instances.len()
    }

    /// Whether a handle still refers to a live instance.
    pub fn is_active(&self, h: InstanceHandle) -> bool {
        self.instances.iter().any(|i| i.id == h.0)
    }
}

impl AudioSource for Engine {
    fn fill(&mut self, out: &mut [f32]) -> usize {
        let frames = out.len() / 2;
        out.fill(0.0);
        if frames == 0 {
            return 0;
        }
        if self.buf_a.len() < out.len() {
            self.buf_a.resize(out.len(), 0.0);
            self.buf_b.resize(out.len(), 0.0);
        }
        let (a, b) = (&mut self.buf_a[..out.len()], &mut self.buf_b[..out.len()]);

        for inst in self.instances.iter_mut() {
            // Player::fill writes the whole block (silence past a one-shot's end)
            // and flips `playing` off when a non-looping instance is exhausted.
            inst.player.fill(a);
            if let Some((out_player, _)) = inst.fading_in.as_mut() {
                out_player.fill(b);
            }
            for f in 0..frames {
                let g = inst.gain.tick();
                let (lg, rg) = balance(inst.pan.tick());
                let (mut l, mut r) = (a[f * 2], a[f * 2 + 1]);
                if let Some((_, mix)) = inst.fading_in.as_mut() {
                    let w = mix.tick();
                    l = l * w + b[f * 2] * (1.0 - w);
                    r = r * w + b[f * 2 + 1] * (1.0 - w);
                }
                out[f * 2] += l * g * lg;
                out[f * 2 + 1] += r * g * rg;
            }
            if let Some((_, mix)) = &inst.fading_in
                && mix.at_target()
            {
                inst.fading_in = None; // crossfade complete
            }
        }

        // Cull finished one-shots and faded-out stops.
        self.instances
            .retain(|i| i.player.playing && !(i.stopping && i.gain.at_target()));
        frames
    }
}

/// A wait-free single-producer / single-consumer ring of `f32` samples. Each
/// slot is an `AtomicU32` (the sample's bits), so it is entirely safe — no
/// `unsafe`, no locks. The [`Controller`] is the sole producer, the [`Renderer`]
/// the sole consumer.
struct SampleRing {
    buf: Vec<AtomicU32>,
    /// Next slot the consumer reads.
    head: AtomicUsize,
    /// Next slot the producer writes.
    tail: AtomicUsize,
}

impl SampleRing {
    fn new(capacity: usize) -> Self {
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

    fn len(&self) -> usize {
        let t = self.tail.load(Ordering::Acquire);
        let h = self.head.load(Ordering::Acquire);
        (t + self.cap() - h) % self.cap()
    }

    fn free(&self) -> usize {
        self.cap() - 1 - self.len()
    }

    /// Push one sample (producer). Returns false if full.
    fn push(&self, sample: f32) -> bool {
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
    fn pop(&self) -> Option<f32> {
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
    ring: Arc<SampleRing>,
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

impl Engine {
    /// Split into a [`Controller`] (control thread) and a [`Renderer`] (audio
    /// thread) joined by a wait-free ring `ring_frames` deep. Pump the controller
    /// off the audio thread; the renderer drains it in the callback.
    pub fn split(self, ring_frames: usize) -> (Controller, Renderer) {
        spsc(self, ring_frames)
    }
}

/// An [`AudioSource`] over the stateful [`StreamGraph`]: streams a streamable
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

/// Handle to a source added to a [`Mixer`].
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct SourceId(u64);

/// Blanket-implemented for every source, so a [`Mixer`] can hand back a typed
/// `&mut` to a source it owns without forcing `Any` onto the public
/// [`AudioSource`] trait (every plain `fill`-only adapter stays unencumbered).
trait AnySource: AudioSource + std::any::Any {}
impl<T: AudioSource + 'static> AnySource for T {}

struct MixedSource {
    id: u64,
    source: Box<dyn AnySource + Send>,
    gain: f32,
}

/// A simple additive stereo mixer of audio sources — instruments, the SFX
/// [`Engine`], [`StreamSource`]s — each with its own gain. It is itself an
/// [`AudioSource`], so it feeds one output callback (or nests). This is the
/// top-level bus of an arrangement: one instrument per part, plus SFX.
pub struct Mixer {
    sources: Vec<MixedSource>,
    next_id: u64,
    scratch: Vec<f32>,
}

impl Default for Mixer {
    fn default() -> Self {
        Mixer::new()
    }
}

impl Mixer {
    /// An empty mixer.
    pub fn new() -> Self {
        Mixer {
            sources: Vec::new(),
            next_id: 1,
            scratch: Vec::new(),
        }
    }

    /// Add a source at unity gain; returns its handle.
    pub fn add(&mut self, source: impl AudioSource + Send + 'static) -> SourceId {
        let id = self.next_id;
        self.next_id += 1;
        self.sources.push(MixedSource {
            id,
            source: Box::new(source),
            gain: 1.0,
        });
        SourceId(id)
    }

    /// Set a source's gain (no-op for an unknown handle).
    pub fn set_gain(&mut self, id: SourceId, gain: f32) {
        if let Some(s) = self.sources.iter_mut().find(|s| s.id == id.0) {
            s.gain = gain.max(0.0);
        }
    }

    /// Mutable access to an added source, downcast to its concrete type — e.g. to
    /// call [`Instrument::note_on`](crate::instrument::Instrument::note_on).
    pub fn get_mut<T: AudioSource + 'static>(&mut self, id: SourceId) -> Option<&mut T> {
        let s = self.sources.iter_mut().find(|s| s.id == id.0)?;
        let any: &mut dyn std::any::Any = s.source.as_mut();
        any.downcast_mut::<T>()
    }

    /// Remove a source.
    pub fn remove(&mut self, id: SourceId) {
        self.sources.retain(|s| s.id != id.0);
    }

    /// Number of sources in the mix.
    pub fn source_count(&self) -> usize {
        self.sources.len()
    }

    /// Whether `id` still refers to a source in the mix.
    pub fn contains(&self, id: SourceId) -> bool {
        self.sources.iter().any(|s| s.id == id.0)
    }
}

impl AudioSource for Mixer {
    fn fill(&mut self, out: &mut [f32]) -> usize {
        let frames = out.len() / 2;
        out.fill(0.0);
        if self.scratch.len() < out.len() {
            self.scratch.resize(out.len(), 0.0);
        }
        let scratch = &mut self.scratch[..out.len()];
        for s in self.sources.iter_mut() {
            s.source.fill(scratch);
            for (o, &x) in out.iter_mut().zip(scratch.iter()) {
                *o += x * s.gain;
            }
        }
        frames
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn doc(duration: f32) -> SoundDoc {
        serde_json::from_str(&format!(
            r#"{{ "name": "t", "duration": {duration}, "root": {{ "type": "sine", "freq": 440 }} }}"#
        ))
        .unwrap()
    }

    fn pitch_patch() -> Patch {
        serde_json::from_str(
            r#"{ "doc": { "name": "t", "duration": 0.5, "root": { "type": "sine", "freq": 440 } },
                 "params": [ { "name": "pitch", "paths": ["root.freq"], "min": 100, "max": 2000, "default": 440 } ] }"#,
        )
        .unwrap()
    }

    fn two_layer_doc() -> SoundDoc {
        serde_json::from_str(
            r#"{ "name": "m", "duration": 0.5, "root": { "type": "tracks", "tracks": [
                   { "id": "bass", "node": { "type": "sine", "freq": 110 } },
                   { "id": "arp",  "node": { "type": "sine", "freq": 880 } }
                 ] } }"#,
        )
        .unwrap()
    }

    fn peak(buf: &[f32]) -> f32 {
        buf.iter().fold(0.0f32, |m, &x| m.max(x.abs()))
    }

    #[test]
    fn one_patch_spawns_many_independent_instances() {
        let mut e = Engine::new(44_100);
        let p = e.load(&doc(1.0));
        let _a = e.play(p);
        let _b = e.play(p);
        let _c = e.play(p);
        assert_eq!(
            e.active(),
            3,
            "resource → instance: many instances of one patch"
        );

        let mut out = vec![0.0f32; 512 * 2];
        assert_eq!(e.fill(&mut out), 512);
        assert!(peak(&out) > 0.0, "the mix should produce audio");
    }

    #[test]
    fn gain_tween_ramps_to_silence() {
        let mut e = Engine::new(1000);
        let p = e.load(&doc(1.0));
        let h = e.play(p);
        e.set_gain(h, 0.0, Tween::frames(100));
        let mut out = vec![0.0f32; 50 * 2];
        e.fill(&mut out);
        let mut rest = vec![0.0f32; 100 * 2];
        e.fill(&mut rest);
        assert!(
            peak(&rest[80 * 2..]) < 1e-3,
            "gain reached 0 after the tween"
        );
    }

    #[test]
    fn stop_declicks_and_culls_the_instance() {
        let mut e = Engine::new(44_100);
        let p = e.load(&doc(5.0));
        let h = e.play_looping(p);
        assert!(e.is_active(h));
        e.stop(h, Tween::ms(10.0, 44_100));
        let mut out = vec![0.0f32; 1024 * 2];
        e.fill(&mut out);
        e.fill(&mut out);
        assert!(!e.is_active(h), "stopped instance is culled once silent");
    }

    #[test]
    fn one_shot_culls_itself_at_end() {
        let mut e = Engine::new(1000);
        let p = e.load(&doc(0.1));
        e.play(p);
        assert_eq!(e.active(), 1);
        let mut out = vec![0.0f32; 256 * 2];
        e.fill(&mut out);
        assert_eq!(e.active(), 0, "a finished one-shot removes itself");
    }

    #[test]
    fn param_resolves_and_set_param_keeps_it_playing() {
        let mut e = Engine::new(44_100);
        let p = e.load_patch(&pitch_patch());
        let pitch = e.param(p, "pitch").expect("pitch param");
        assert!(e.param(p, "nope").is_none());
        let h = e.play_looping(p);

        let mut out = vec![0.0f32; 256 * 2];
        e.fill(&mut out);
        e.set_param(h, pitch, 880.0, Tween::ms(5.0, 44_100));
        // Crossfade in progress: still exactly one live instance, still audible.
        assert_eq!(e.active(), 1);
        let mut out2 = vec![0.0f32; 1024 * 2];
        e.fill(&mut out2);
        assert!(peak(&out2) > 0.0, "still playing at the new pitch");
    }

    #[test]
    fn layer_resolves_and_gain_change_is_click_free() {
        let mut e = Engine::new(44_100);
        let p = e.load(&two_layer_doc());
        let arp = e.layer(p, "arp").expect("arp layer");
        assert!(e.layer(p, "missing").is_none());
        let h = e.play_looping(p);
        let mut out = vec![0.0f32; 256 * 2];
        e.fill(&mut out);
        e.set_layer_gain(h, arp, 0.0, Tween::ms(20.0, 44_100));
        e.fill(&mut out);
        assert!(e.is_active(h), "layer move does not drop the instance");
    }

    #[test]
    fn hard_pan_silences_the_opposite_channel() {
        let (l, r) = balance(1.0); // +1 = hard right
        assert!(l.abs() < 1e-6 && (r - 1.0).abs() < 1e-6);
        let (l, r) = balance(-1.0); // -1 = hard left
        assert!((l - 1.0).abs() < 1e-6 && r.abs() < 1e-6);
        let (l, r) = balance(0.0);
        assert!(
            (l - 1.0).abs() < 1e-6 && (r - 1.0).abs() < 1e-6,
            "unity at centre"
        );
    }

    #[test]
    fn ring_pushes_pops_and_wraps() {
        let r = SampleRing::new(4); // 4 usable slots
        assert!(r.pop().is_none());
        for i in 0..4 {
            assert!(r.push(i as f32));
        }
        assert!(!r.push(9.0), "full");
        assert_eq!(r.pop(), Some(0.0));
        assert!(r.push(9.0), "space freed after a pop");
        let got: Vec<f32> = std::iter::from_fn(|| r.pop()).collect();
        assert_eq!(got, vec![1.0, 2.0, 3.0, 9.0]);
    }

    #[test]
    fn split_pumps_audio_across_the_seam() {
        let mut e = Engine::new(44_100);
        let p = e.load(&doc(1.0));
        let (mut ctl, mut rend) = e.split(1024);
        ctl.play_looping(p); // Deref → Engine::play_looping
        assert!(ctl.pump(512) > 0, "controller produced frames");
        let mut out = vec![0.0f32; 512 * 2];
        assert_eq!(rend.fill(&mut out), 512);
        assert!(peak(&out) > 0.0, "renderer drained real audio");
    }

    #[test]
    fn spsc_pumps_a_mixer_across_the_seam() {
        // The generalized seam drives a whole Mixer, not just an Engine — the
        // shape the Python owned-stream binding pumps.
        let mut engine = Engine::new(44_100);
        let p = engine.load(&doc(1.0));
        engine.play_looping(p);
        let mut mixer = Mixer::new();
        mixer.add(engine);
        let (mut ctl, mut rend) = spsc(mixer, 1024);
        assert!(ctl.pump(512) > 0, "pump produced frames");
        assert_eq!(ctl.source_count(), 1, "deref reaches the Mixer");
        let mut out = vec![0.0f32; 512 * 2];
        assert_eq!(rend.fill(&mut out), 512);
        assert!(peak(&out) > 0.0, "renderer drained real audio");
    }

    #[test]
    fn pump_never_drops_rendered_frames() {
        // The split path must deliver the same bytes as an unsplit engine:
        // pumping more than the ring can take must not advance play heads
        // past what was actually delivered.
        let mk = || {
            let mut e = Engine::new(44_100);
            let p = e.load(&doc(1.0));
            e.play_looping(p);
            e
        };
        let mut reference = mk();
        let mut expected = vec![0.0f32; 192 * 2];
        reference.fill(&mut expected);

        let (mut ctl, mut rend) = mk().split(64);
        let mut got = Vec::new();
        let mut out = vec![0.0f32; 64 * 2];
        while got.len() < expected.len() {
            ctl.pump(200); // over-ask: the ring only holds 64 frames
            rend.fill(&mut out);
            got.extend_from_slice(&out);
        }
        assert_eq!(
            &got[..expected.len()],
            &expected[..],
            "over-pumping dropped rendered frames"
        );
    }

    #[test]
    fn renderer_drains_whole_frames_only() {
        // A partial frame in the ring must not shift channel alignment.
        let ring = SampleRing::new(8);
        for s in [1.0f32, 2.0, 3.0] {
            ring.push(s); // one and a half frames
        }
        let mut rend = Renderer {
            ring: std::sync::Arc::new(ring),
        };
        let mut out = vec![9.0f32; 4];
        rend.fill(&mut out);
        assert_eq!(out, vec![1.0, 2.0, 0.0, 0.0], "half frame must stay queued");
        rend.ring.push(4.0);
        let mut out = vec![9.0f32; 2];
        rend.fill(&mut out);
        assert_eq!(
            out,
            vec![3.0, 4.0],
            "queued half frame pairs with the next sample"
        );
    }

    #[test]
    fn renderer_underrun_writes_silence() {
        let e = Engine::new(44_100);
        let (_ctl, mut rend) = e.split(256); // nothing pumped
        let mut out = vec![1.0f32; 128 * 2];
        rend.fill(&mut out);
        assert!(peak(&out) < 1e-9, "underrun is clean silence, not garbage");
    }

    #[test]
    fn control_and_audio_sides_are_send() {
        fn assert_send<T: Send>() {}
        assert_send::<Controller>();
        assert_send::<Renderer>();
    }

    #[test]
    fn stream_source_streams_a_streamable_doc() {
        let d: SoundDoc = serde_json::from_str(
            r#"{ "name":"s", "duration":0.1, "root": { "type":"chain", "stages": [
                { "type":"sawtooth", "freq":220 },
                { "type":"lowpass", "cutoff":900, "q":0.7 } ] } }"#,
        )
        .unwrap();
        let mut src = StreamSource::from_doc(&d).expect("streamable");
        let mut out = vec![0.0f32; 256 * 2];
        assert_eq!(src.fill(&mut out), 256);
        assert!(peak(&out) > 0.0, "streams real audio");
        // Mono duplicated to stereo: channels are identical.
        assert!((0..256).all(|f| out[f * 2] == out[f * 2 + 1]));
    }

    #[test]
    fn stream_source_matches_the_bounce_including_its_peak_limit() {
        // A full-scale sine peaks above the 0.989 ceiling, so the offline
        // bounce attenuates it. The stream must carry the identical gain or
        // it plays louder than the bounce and can clip the DAC.
        let d: SoundDoc = serde_json::from_str(
            r#"{ "name":"loud", "duration":0.1, "root": { "type":"sine", "freq":220 } }"#,
        )
        .unwrap();
        let bounce = crate::render::render(&d);
        let mut src = StreamSource::from_doc(&d).expect("streamable");
        let mut out = vec![0.0f32; bounce.len() * 2];
        src.fill(&mut out);
        for (i, b) in bounce.iter().enumerate() {
            assert_eq!(
                out[i * 2].to_bits(),
                b.to_bits(),
                "stream diverges from the bounce at sample {i}"
            );
        }
    }

    #[test]
    fn stream_source_rejects_non_streamable() {
        let d: SoundDoc = serde_json::from_str(
            r#"{ "name":"n", "duration":0.05, "root": { "type":"noise", "color":"white" } }"#,
        )
        .unwrap();
        assert!(StreamSource::from_doc(&d).is_none());
    }

    #[test]
    fn mixer_sums_and_reaches_in_by_type() {
        let mut e = Engine::new(44_100);
        let p = e.load(&doc(1.0));
        e.play_looping(p);
        let mut mixer = Mixer::new();
        let id = mixer.add(e);
        assert_eq!(mixer.source_count(), 1);
        // Reach back into the owned Engine and spawn another instance.
        mixer.get_mut::<Engine>(id).unwrap().play_looping(p);
        assert_eq!(mixer.get_mut::<Engine>(id).unwrap().active(), 2);
        mixer.set_gain(id, 0.5);
        let mut out = vec![0.0f32; 256 * 2];
        assert_eq!(mixer.fill(&mut out), 256);
        assert!(peak(&out) > 0.0, "mixer sums its sources");
        mixer.remove(id);
        assert_eq!(mixer.source_count(), 0);
    }
}
