//! The [`Engine`]: patch resources, live instances, polyphony/voice-stealing,
//! and per-instance parameter re-renders.

use std::collections::BTreeMap;

use super::SCRATCH_FRAMES;
use super::ramp::{Ramp, Tween};
use super::ring::{Controller, Renderer, spsc};
use super::source::AudioSource;
use crate::dsl::{Node, SoundDoc};
use crate::edit::{EditOp, apply_ops};
use crate::patch::Patch;
use crate::player::Player;

/// Fade applied wherever an abrupt gain change would click: voice steals and
/// zero-length stops.
const DECLICK_MS: f32 = 5.0;
/// Floor for the re-render crossfade — a zero-length swap would click.
const CROSSFADE_MIN_MS: f32 = 8.0;

/// Handle to a loaded patch — an immutable, shareable resource. Cheap to copy;
/// spawn as many instances of it as you like.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct PatchId(pub(crate) usize);

/// Handle to one live instance of a patch. Stable for the instance's lifetime;
/// setters against a finished/unknown handle are no-ops.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct InstanceHandle(pub(super) u64);

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

/// A voice's importance under a polyphony cap: when the [`Engine`] is at its
/// [`max_voices`](Engine::set_max_voices) budget, a new voice steals the
/// lowest-priority sounding one (oldest first on a tie), and is itself denied if
/// every voice outranks it. Higher wins. Use the named tiers or any `u8`.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct Priority(pub u8);

impl Priority {
    /// Ambient, interruptible one-shots (footsteps, UI ticks). `0`.
    pub const LOW: Priority = Priority(0);
    /// The default for [`Engine::play`]. `64`.
    pub const NORMAL: Priority = Priority(64);
    /// Salient events that should win a voice (impacts, pickups). `128`.
    pub const HIGH: Priority = Priority(128);
    /// Never stolen while anything lower is sounding (music, stingers). `255`.
    pub const CRITICAL: Priority = Priority(255);
}

impl Default for Priority {
    fn default() -> Self {
        Priority::NORMAL
    }
}

/// How [`Engine::spawn`] starts an instance. The default is a one-shot at
/// [`Priority::NORMAL`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Spawn {
    /// Loop until stopped instead of playing once.
    pub looping: bool,
    /// Importance under a polyphony cap.
    pub priority: Priority,
}

/// A voice captured from a patch by [`Engine::prepare`], to be rendered off
/// the engine's lock and handed back to [`Engine::spawn_prepared`].
///
/// A real-time host keeps its `Engine` behind a lock the audio callback only
/// `try_lock`s, so rendering a long one-shot inside [`spawn`](Engine::spawn)
/// silences the whole output for the length of the render. Splitting the
/// spawn moves that render out: `prepare` and `spawn_prepared` only copy and
/// insert, and [`render`](Self::render) runs on any thread. Cloning a rendered
/// voice shares its audio, so a game can keep one per sound and replay it
/// without rendering again.
#[derive(Clone)]
pub struct PreparedVoice {
    patch: usize,
    sample_rate: u32,
    values: BTreeMap<String, f32>,
    state: VoiceState,
}

#[derive(Clone)]
enum VoiceState {
    /// The patch instantiated at its defaults, at the engine's rate.
    Doc(Box<SoundDoc>),
    Rendered(Player),
}

impl PreparedVoice {
    /// Render the voice's audio now, if it is not rendered yet. This is the
    /// O(duration) step; call it wherever a render may block.
    pub fn render(&mut self) {
        if let VoiceState::Doc(doc) = &self.state {
            let player = Player::new((**doc).clone());
            self.state = VoiceState::Rendered(player);
        }
    }

    /// Whether [`render`](Self::render) has run.
    pub fn is_rendered(&self) -> bool {
        matches!(self.state, VoiceState::Rendered(_))
    }

    /// The patch this voice plays.
    pub fn patch(&self) -> PatchId {
        PatchId(self.patch)
    }
}

/// Equal-level stereo balance (unity at centre): `pan` −1 = hard left, +1 = hard
/// right. Returns per-channel gains `(left, right)`.
pub(super) fn balance(pan: f32) -> (f32, f32) {
    let l = if pan <= 0.0 { 1.0 } else { 1.0 - pan };
    let r = if pan >= 0.0 { 1.0 } else { 1.0 + pan };
    (l, r)
}

pub(super) struct Instance {
    pub(super) id: u64,
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
    pub(super) stopping: bool,
    /// Importance under a polyphony cap (higher survives a steal).
    priority: Priority,
}

/// The runtime mixer: owns patch resources and their live instances, and serves
/// their mixed-down stereo through [`AudioSource::fill`].
///
/// ```
/// use tono_core::prelude::*;
/// use tono_core::dsl::Node;
///
/// let doc = SoundDoc::new("blip", Node::Sine { freq: 880.0.into() });
/// let mut engine = Engine::new(48_000);
/// let blip = engine.load(&doc);          // a reusable resource…
/// let _voice = engine.play(blip);        // …spawning independent instances
///
/// let mut out = vec![0.0f32; 512];       // interleaved stereo L,R,L,R…
/// engine.fill(&mut out);
/// assert!(out.iter().any(|s| s.abs() > 0.0), "the blip is sounding");
/// ```
/// # Threading
/// Mutating calls ([`play`](Self::play), [`set_param`](Self::set_param),
/// [`stop`](Self::stop), …) render the whole document synchronously —
/// O(duration), with allocations — so they must never run on an audio
/// callback thread. Real-time hosts use [`split`](Self::split) to render on a
/// pump thread joined to the callback by a wait-free ring, or keep the engine
/// behind a lock and render one-shots outside it with
/// [`prepare`](Self::prepare) and [`spawn_prepared`](Self::spawn_prepared). Note that
/// `set_param` re-renders the whole doc per call with no coalescing —
/// rate-limit parameter automation on the control side.
pub struct Engine {
    sample_rate: u32,
    patches: Vec<Patch>,
    pub(super) instances: Vec<Instance>,
    next_id: u64,
    /// Optional polyphony budget; `None` = unlimited (the default).
    max_voices: Option<usize>,
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
            max_voices: None,
            buf_a: vec![0.0; SCRATCH_FRAMES * 2],
            buf_b: vec![0.0; SCRATCH_FRAMES * 2],
        }
    }

    /// Cap the number of concurrently sounding instances. Once at the budget, a
    /// new [`play`](Self::play) steals the lowest-[`Priority`] sounding instance
    /// (declicked, not hard-cut) — or is denied if every instance outranks it.
    /// Unset by default (unlimited). A `max` of 0 is treated as 1.
    pub fn set_max_voices(&mut self, max: usize) {
        self.max_voices = Some(max.max(1));
    }

    /// The current polyphony budget, or `None` if unlimited.
    pub fn max_voices(&self) -> Option<usize> {
        self.max_voices
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
        // .get(): a PatchId is Copy and could come from another Engine — an
        // unknown handle resolves to None, never a panic.
        self.patches
            .get(patch.0)?
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
        match &self.patches.get(patch.0)?.doc.root {
            Node::Tracks { tracks, .. } => tracks
                .iter()
                .position(|t| t.id.as_deref() == Some(name))
                .map(|index| LayerId {
                    patch: patch.0,
                    index,
                }),
            _ => None,
        }
    }

    /// Spawn a one-shot instance of a patch (plays once, then culls itself) at
    /// [`Priority::NORMAL`].
    pub fn play(&mut self, patch: PatchId) -> InstanceHandle {
        self.spawn(patch, Spawn::default())
    }

    /// Spawn a looping instance of a patch (plays until [`stop`](Self::stop)ped)
    /// at [`Priority::NORMAL`].
    pub fn play_looping(&mut self, patch: PatchId) -> InstanceHandle {
        self.spawn(
            patch,
            Spawn {
                looping: true,
                ..Spawn::default()
            },
        )
    }

    /// Spawn an instance of a patch with explicit options — looping or
    /// one-shot, and its [`Priority`] under a voice cap. A higher priority
    /// survives a steal; a voice outranked by every sounding instance is
    /// denied and returns an inert handle ([`is_active`](Self::is_active) is
    /// `false`). [`play`](Self::play) and [`play_looping`](Self::play_looping)
    /// are the common cases. Renders the patch here; a host that holds the
    /// engine under a lock uses [`prepare`](Self::prepare) instead.
    pub fn spawn(&mut self, patch: PatchId, how: Spawn) -> InstanceHandle {
        match self.prepare(patch) {
            Some(voice) => self.spawn_prepared(voice, how),
            // .get() in prepare: a PatchId is Copy and could come from another
            // Engine — return the inert handle rather than panic.
            None => InstanceHandle(0),
        }
    }

    /// Capture a patch at its default parameters for a later
    /// [`spawn_prepared`](Self::spawn_prepared), without rendering it. `None`
    /// if the patch is not loaded in this engine.
    pub fn prepare(&self, patch: PatchId) -> Option<PreparedVoice> {
        let values = self.patches.get(patch.0).map(Patch::defaults)?;
        let mut doc = self.build_doc(patch.0, &values, &BTreeMap::new());
        // Instances render at the engine's rate, not each doc's own — one internal
        // rate for the whole mix (resampling to the device belongs at the adapter).
        doc.sample_rate = self.sample_rate;
        Some(PreparedVoice {
            patch: patch.0,
            sample_rate: self.sample_rate,
            values,
            state: VoiceState::Doc(Box::new(doc)),
        })
    }

    /// Spawn a voice from [`prepare`](Self::prepare), rendering it first if
    /// [`PreparedVoice::render`] has not run. Behaves as [`spawn`](Self::spawn)
    /// does, including a denial under the voice cap; a voice prepared at
    /// another sample rate, or for a patch this engine never loaded, returns
    /// the inert handle.
    pub fn spawn_prepared(&mut self, voice: PreparedVoice, how: Spawn) -> InstanceHandle {
        let Spawn { looping, priority } = how;
        if voice.sample_rate != self.sample_rate || voice.patch >= self.patches.len() {
            return InstanceHandle(0);
        }
        // Enforce the polyphony budget (if any) before adding a voice.
        if let Some(max) = self.max_voices
            && !self.make_room(max, priority)
        {
            // Outranked by every sounding voice — deny (virtualize to silence).
            return InstanceHandle(0);
        }
        let mut player = match voice.state {
            VoiceState::Doc(doc) => Player::new(*doc),
            VoiceState::Rendered(player) => player,
        };
        player.looping = looping;
        player.play();
        let id = self.next_id;
        self.next_id += 1;
        self.instances.push(Instance {
            id,
            patch: voice.patch,
            values: voice.values,
            layer_gains: BTreeMap::new(),
            player,
            fading_in: None,
            gain: Ramp::new(1.0),
            pan: Ramp::new(0.0),
            stopping: false,
            priority,
        });
        InstanceHandle(id)
    }

    /// Make room for a new voice of `priority` under a `max` budget. Returns
    /// `false` (deny the new voice) if every sounding instance outranks it.
    /// Mirrors the instrument's two-stage bound: a soft steal declicks the victim
    /// (so it briefly rides on top during the fade), and a hard bound at `2*max`
    /// drops a voice outright so a flood faster than the declick can't grow.
    fn make_room(&mut self, max: usize, priority: Priority) -> bool {
        let sounding = self.instances.iter().filter(|i| !i.stopping).count();
        if sounding >= max {
            // Victim = lowest priority, oldest (lowest id) on a tie.
            let victim = self
                .instances
                .iter()
                .filter(|i| !i.stopping)
                .min_by(|a, b| a.priority.cmp(&b.priority).then(a.id.cmp(&b.id)))
                .map(|i| (i.id, i.priority));
            match victim {
                Some((id, vp)) if vp <= priority => {
                    let fade = Tween::ms(DECLICK_MS, self.sample_rate);
                    if let Some(v) = self.instances.iter_mut().find(|i| i.id == id) {
                        v.gain.set(0.0, fade);
                        v.stopping = true;
                    }
                }
                _ => return false,
            }
        }
        // Hard cap so declicking victims can't grow the Vec without bound.
        if self.instances.len() >= max * 2
            && let Some(pos) = self
                .instances
                .iter()
                .enumerate()
                .min_by(|(_, a), (_, b)| a.priority.cmp(&b.priority).then(a.id.cmp(&b.id)))
                .map(|(pos, _)| pos)
        {
            self.instances.remove(pos);
        }
        true
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
            // clamp() passes NaN through, and a NaN pan would NaN the whole
            // mix — fold it to centre.
            let pan = if pan.is_nan() {
                0.0
            } else {
                pan.clamp(-1.0, 1.0)
            };
            i.pan.set(pan, tw);
        }
    }

    /// Set a named parameter on a live instance, crossfading over `tw` for a
    /// click-free swap. `param` must come from the same patch the instance plays.
    pub fn set_param(&mut self, h: InstanceHandle, param: ParamId, value: f32, tw: Tween) {
        let Some(idx) = self.find(h) else { return };
        if self.instances[idx].patch != param.patch {
            return;
        }
        // .get(): a ParamId is Copy and could come from another Engine whose
        // same-indexed patch has more params — never panic.
        let Some(name) = self
            .patches
            .get(param.patch)
            .and_then(|p| p.params.get(param.index))
            .map(|p| p.name.clone())
        else {
            return;
        };
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

        let fade = if tw.is_immediate() {
            Tween::ms(CROSSFADE_MIN_MS, self.sample_rate)
        } else {
            tw
        };
        let inst = &mut self.instances[idx];
        // The current player becomes the outgoing one; the fresh render fades in.
        let outgoing = std::mem::replace(&mut inst.player, fresh);
        // A stacked re-render (a param change landing mid-crossfade) drops the
        // still-fading previous outgoing player. Starting the new mix at the
        // old mix's current weight keeps the blend monotonic instead of
        // re-fading from scratch, so the dropped tail carries only the
        // remaining (1 − w) of its energy.
        let w0 = inst.fading_in.as_ref().map_or(0.0, |(_, m)| m.value());
        let mut mix = Ramp::new(w0);
        mix.set(1.0, fade);
        inst.fading_in = Some((outgoing, mix));
    }

    /// Stop an instance with a declick fade-out; it is culled once silent. A
    /// zero-length `fade` is bumped to a short default so stops never click.
    pub fn stop(&mut self, h: InstanceHandle, fade: Tween) {
        let min_fade = Tween::ms(DECLICK_MS, self.sample_rate);
        if let Some(i) = self.instance_mut(h) {
            let fade = if fade.is_immediate() { min_fade } else { fade };
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

    /// The current crossfade weight of an instance (test-only introspection).
    #[cfg(test)]
    pub(super) fn fade_weight(&self, idx: usize) -> Option<f32> {
        self.instances
            .get(idx)?
            .fading_in
            .as_ref()
            .map(|(_, m)| m.value())
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

        // Cull finished one-shots and faded-out stops. An instance whose fresh
        // one-shot ended mid-crossfade lives until the fade completes — its
        // outgoing player is still sounding.
        self.instances.retain(|i| {
            let outgoing_sounding = i.fading_in.as_ref().is_some_and(|(p, _)| p.playing);
            (i.player.playing || outgoing_sounding) && !(i.stopping && i.gain.at_target())
        });
        frames
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
