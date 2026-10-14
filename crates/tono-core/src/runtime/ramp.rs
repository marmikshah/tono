//! Control-rate smoothing: the [`Tween`] a caller asks for and the per-frame
//! [`Ramp`] that delivers it — instance faders, the performance's master gain
//! and swap crossfade, and stinger declicks all ride the same one.

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

    /// Whether the tween jumps instantly.
    pub(crate) fn is_immediate(&self) -> bool {
        self.frames == 0
    }
}

impl Default for Tween {
    fn default() -> Self {
        Tween::IMMEDIATE
    }
}

/// A per-frame linear ramp toward a target value.
#[derive(Clone, Copy)]
pub(crate) struct Ramp {
    value: f32,
    target: f32,
    step: f32,
    remaining: u32,
}

impl Ramp {
    pub(crate) fn new(v: f32) -> Self {
        Ramp {
            value: v,
            target: v,
            step: 0.0,
            remaining: 0,
        }
    }

    pub(crate) fn set(&mut self, target: f32, tw: Tween) {
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
    pub(crate) fn tick(&mut self) -> f32 {
        if self.remaining > 0 {
            self.value += self.step;
            self.remaining -= 1;
            if self.remaining == 0 {
                self.value = self.target;
            }
        }
        self.value
    }

    /// The value the ramp is heading for.
    pub(crate) fn target(&self) -> f32 {
        self.target
    }

    /// The value the next frame starts from.
    pub(crate) fn value(&self) -> f32 {
        self.value
    }

    pub(crate) fn at_target(&self) -> bool {
        self.remaining == 0
    }
}
