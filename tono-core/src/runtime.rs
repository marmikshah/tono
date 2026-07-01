//! runtime — the embeddable real-time control surface over the deterministic engine.
//!
//! The idiomatic library API a game (or any host) drives: [`Engine::load`] a
//! [`SoundDoc`] (or [`Engine::load_patch`] a [`Patch`] with named parameters) as
//! a reusable **resource**, [`Engine::play`] as many independent **instances** as
//! you like, and control each by its [`InstanceHandle`] with [`Tween`]-smoothed
//! setters. Host output adapters (cpal, an AudioWorklet, a Bevy source) target
//! the [`AudioSource`] trait, so they never depend on a concrete engine type.
//!
//! Backed today by the deterministic buffer renderer ([`crate::stream::Player`]),
//! which keeps the mix **byte-identical to an offline bounce**. Instance master
//! controls (gain / pan / stop) apply live per block; parameter and layer-gain
//! changes ([`Engine::set_param`] / [`Engine::set_layer_gain`]) re-render the
//! instance and **crossfade** for a click-free swap — control-rate today, and
//! sample-accurate once the stateful streaming renderer lands behind this same
//! seam. Multi-threaded real-time use goes through [`Engine::split`].

use std::collections::BTreeMap;

use crate::dsl::{Node, SoundDoc};
use crate::edit::{EditOp, apply_ops};
use crate::patch::Patch;
use crate::stream::Player;

/// A block-serving audio source: fill `out` (interleaved stereo L,R,L,R…) and
/// return the number of frames written. Runs indefinitely. This is the single
/// seam host output adapters target, so a `cpal` callback, an AudioWorklet, or a
/// Bevy source never depend on a concrete engine type.
pub trait AudioSource {
    /// Fill `out` with the next block of interleaved-stereo audio.
    fn fill(&mut self, out: &mut [f32]) -> usize;
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
            let has_fade = inst.fading_in.is_some();
            if let Some((out_player, _)) = inst.fading_in.as_mut() {
                out_player.fill(b);
            }
            for f in 0..frames {
                let g = inst.gain.tick();
                let (lg, rg) = balance(inst.pan.tick());
                let (mut l, mut r) = (a[f * 2], a[f * 2 + 1]);
                if has_fade {
                    let (_, mix) = inst.fading_in.as_mut().unwrap();
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
}
