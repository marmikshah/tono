//! runtime — the embeddable real-time control surface over the deterministic engine.
//!
//! The idiomatic library API a game (or any host) drives: [`Engine::load`] a
//! [`SoundDoc`] as a reusable **patch resource**, [`Engine::play`] as many
//! independent **instances** as you like, and control each by its
//! [`InstanceHandle`] with [`Tween`]-smoothed setters. Host output adapters
//! (cpal, an AudioWorklet, a Bevy source) target the [`AudioSource`] trait.
//!
//! This first cut is backed by the existing buffer renderer ([`crate::stream::Player`]),
//! which makes the resource→instance model, the mixer, gain/pan, and declicked
//! stop **real-time today**. Per-graph parameter modulation (typed `ParamId`
//! handles) and a lock-free control/audio `split()` arrive with the streaming
//! stateful renderer, behind this same `AudioSource` seam.

use crate::dsl::SoundDoc;
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

struct Instance {
    id: u64,
    player: Player,
    gain: Ramp,
    pan: Ramp,
    /// Fading out to be culled once silent.
    stopping: bool,
}

/// The runtime mixer: owns patch resources and their live instances, and serves
/// their mixed-down stereo through [`AudioSource::fill`].
pub struct Engine {
    sample_rate: u32,
    patches: Vec<SoundDoc>,
    instances: Vec<Instance>,
    next_id: u64,
    scratch: Vec<f32>,
}

impl Engine {
    /// A fresh engine that renders at `sample_rate`.
    pub fn new(sample_rate: u32) -> Self {
        Engine {
            sample_rate,
            patches: Vec::new(),
            instances: Vec::new(),
            next_id: 1,
            scratch: Vec::new(),
        }
    }

    /// The engine's sample rate.
    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    /// Load a document as a reusable patch resource.
    pub fn load(&mut self, doc: &SoundDoc) -> PatchId {
        self.patches.push(doc.clone());
        PatchId(self.patches.len() - 1)
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
        // Instances render at the engine's rate, not each doc's own — one
        // internal rate for the whole mix (resampling to the device belongs at
        // the output adapter, not here).
        let mut doc = self.patches[patch.0].clone();
        doc.sample_rate = self.sample_rate;
        let mut player = Player::new(doc);
        player.looping = looping;
        player.play();
        let id = self.next_id;
        self.next_id += 1;
        self.instances.push(Instance {
            id,
            player,
            gain: Ramp::new(1.0),
            pan: Ramp::new(0.0),
            stopping: false,
        });
        InstanceHandle(id)
    }

    fn instance_mut(&mut self, h: InstanceHandle) -> Option<&mut Instance> {
        self.instances.iter_mut().find(|i| i.id == h.0)
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
        if self.scratch.len() < out.len() {
            self.scratch.resize(out.len(), 0.0);
        }
        let scratch = &mut self.scratch[..out.len()];

        for inst in self.instances.iter_mut() {
            // Player::fill writes the whole block (silence past a one-shot's end)
            // and flips `playing` off when a non-looping instance is exhausted.
            inst.player.fill(scratch);
            for f in 0..frames {
                let g = inst.gain.tick();
                let (lg, rg) = balance(inst.pan.tick());
                out[f * 2] += scratch[f * 2] * g * lg;
                out[f * 2 + 1] += scratch[f * 2 + 1] * g * rg;
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
    fn gain_tween_ramps_toward_the_target() {
        let mut e = Engine::new(1000); // 1000 Hz → easy frame math
        let p = e.load(&doc(1.0));
        let h = e.play(p);
        // Ramp gain 1.0 → 0.0 over 100 frames; after 50 frames it should be ~0.5.
        e.set_gain(h, 0.0, Tween::frames(100));
        let mut out = vec![0.0f32; 50 * 2];
        e.fill(&mut out);
        // Pull the ramp value out by fully finishing it and checking silence.
        let mut rest = vec![0.0f32; 100 * 2];
        e.fill(&mut rest);
        // After the full ramp the instance is at gain 0 (its tail is silent).
        let tail = &rest[80 * 2..];
        assert!(peak(tail) < 1e-3, "gain reached 0 after the tween");
    }

    #[test]
    fn stop_declicks_and_culls_the_instance() {
        let mut e = Engine::new(44_100);
        let p = e.load(&doc(5.0));
        let h = e.play_looping(p);
        assert!(e.is_active(h));
        e.stop(h, Tween::ms(10.0, 44_100));
        // Serve well past the 10 ms fade (441 frames) to let it cull.
        let mut out = vec![0.0f32; 1024 * 2];
        e.fill(&mut out);
        e.fill(&mut out);
        assert!(!e.is_active(h), "stopped instance is culled once silent");
        assert_eq!(e.active(), 0);
    }

    #[test]
    fn one_shot_culls_itself_at_end() {
        let mut e = Engine::new(1000);
        let p = e.load(&doc(0.1)); // 100 ms → 100 frames at 1 kHz
        e.play(p);
        assert_eq!(e.active(), 1);
        let mut out = vec![0.0f32; 256 * 2]; // longer than the sound
        e.fill(&mut out);
        assert_eq!(e.active(), 0, "a finished one-shot removes itself");
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
