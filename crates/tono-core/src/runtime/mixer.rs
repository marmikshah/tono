//! The routing [`Mixer`]: input/FX buses with insert chains, faders, and
//! post-fader sends over any set of [`AudioSource`]s.

use super::SCRATCH_FRAMES;
use super::source::AudioSource;
use crate::dsl::{ENGINE_VERSION, Node};
use crate::streaming::EffectChain;

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
    /// The bus this source feeds ([`BusId::MASTER`] by default).
    bus: BusId,
}

/// Handle to a bus in a [`Mixer`] — an input group or an FX/return bus. The
/// master bus is always [`BusId::MASTER`]. The handle's value is the bus's index
/// in the mixer, so it is stable for the mixer's life.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct BusId(u32);

impl BusId {
    /// The always-present master bus. Every source, dry bus output, and FX
    /// return sums here, and the master insert chain is the final stage.
    pub const MASTER: BusId = BusId(0);
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum BusKind {
    Master,
    Input,
    Fx,
}

/// Why a [`Mixer`] bus operation failed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MixerError {
    /// An effect node is outside the real-time-streamable subset.
    NotStreamable,
    /// The mixer has no sample rate. Vestigial — every current constructor
    /// takes the rate up front, so this is unreachable today. Deleted at 2.0.
    #[deprecated(
        since = "1.9.0",
        note = "unreachable: every Mixer constructor takes the rate up front; deleted at 2.0"
    )]
    NoSampleRate,
    /// The bus handle names no live bus (foreign or stale).
    UnknownBus,
}

impl std::fmt::Display for MixerError {
    // The deprecated variant is still displayed until it is deleted at 2.0.
    #[allow(deprecated)]
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MixerError::NotStreamable => {
                write!(f, "effect chain contains a non-streamable node")
            }
            MixerError::NoSampleRate => {
                write!(f, "mixer has no sample rate; build it with Mixer::new")
            }
            MixerError::UnknownBus => {
                write!(f, "unknown bus (foreign or stale BusId)")
            }
        }
    }
}

impl std::error::Error for MixerError {}

/// One bus: a summing point with an optional insert chain, a fader, post-fader
/// sends to FX buses, and a dry level into master.
struct Bus {
    name: String,
    kind: BusKind,
    gain: f32,
    /// Dry level into master (0 = send-only). Unused by the master bus.
    to_master: f32,
    /// A stereo insert chain (identical coefficients, independent L/R state).
    inserts: Option<(EffectChain, EffectChain)>,
    /// Post-fader sends into FX buses, as `(target bus index, level)`.
    sends: Vec<(u32, f32)>,
}

/// A routing stereo mixer of audio sources — instruments, the SFX [`Engine`](super::Engine),
/// [`StreamSource`](super::StreamSource)s. Each source feeds a **bus**; buses carry live insert
/// chains (reverb / EQ / compressor / …) and post-fader **sends** into shared
/// FX/return buses, all summing through a **master** insert chain. It is itself
/// an [`AudioSource`], so it feeds one output callback (or nests).
///
/// With no buses or effects created, every source sits on the master bus at unity
/// and the output is a plain additive sum — byte-identical to a bare mixer. Live
/// effects need a sample rate: build with [`Mixer::new`].
/// ```
/// use tono_core::prelude::*;
/// use tono_core::dsl::Node;
///
/// let mut mixer = Mixer::new(48_000);
/// let sfx = mixer.bus("sfx");
///
/// let mut engine = Engine::new(48_000);
/// let drone = engine.load(&SoundDoc::new("drone", Node::Sine { freq: 110.0.into() }));
/// engine.play_looping(drone);
/// mixer.add_to(sfx, engine);             // the engine feeds the sfx bus
/// mixer.set_bus_gain(sfx, 0.8);          // a live fader
///
/// let mut out = vec![0.0f32; 512];
/// mixer.fill(&mut out);                  // one callback serves the whole desk
/// assert!(out.iter().any(|s| s.abs() > 0.0));
/// ```
pub struct Mixer {
    sources: Vec<MixedSource>,
    /// `buses[0]` is always the master bus; a bus's index equals its [`BusId`].
    buses: Vec<Bus>,
    next_id: u64,
    sample_rate: Option<u32>,
    // Reused planar scratch (grown lazily, never shrunk).
    scratch: Vec<f32>,
    master_l: Vec<f32>,
    master_r: Vec<f32>,
    bus_l: Vec<f32>,
    bus_r: Vec<f32>,
    /// Per-bus FX input accumulators (indexed by bus index; only FX slots used).
    fx_in: Vec<(Vec<f32>, Vec<f32>)>,
}

impl Mixer {
    /// An empty mixer that renders at `sample_rate`, so buses can carry live
    /// effect chains. (Every other runtime constructor — `Engine::new`,
    /// `AdaptiveMusic::new`, `Instrument::new` — takes the rate up front; a
    /// rate-less mixer deferred the failure to the first `fx_bus` call.)
    pub fn new(sample_rate: u32) -> Self {
        Mixer::build(Some(sample_rate))
    }

    fn build(sample_rate: Option<u32>) -> Self {
        let master = Bus {
            name: "master".into(),
            kind: BusKind::Master,
            gain: 1.0,
            to_master: 1.0,
            inserts: None,
            sends: Vec::new(),
        };
        Mixer {
            sources: Vec::new(),
            buses: vec![master],
            next_id: 1,
            sample_rate,
            // Pre-sized so common host blocks never allocate in `fill` (the
            // grow() calls stay as the fallback for bigger ones).
            scratch: vec![0.0; SCRATCH_FRAMES * 2],
            master_l: vec![0.0; SCRATCH_FRAMES],
            master_r: vec![0.0; SCRATCH_FRAMES],
            bus_l: vec![0.0; SCRATCH_FRAMES],
            bus_r: vec![0.0; SCRATCH_FRAMES],
            fx_in: Vec::new(),
        }
    }

    /// Add a source to the master bus at unity gain; returns its handle.
    pub fn add(&mut self, source: impl AudioSource + Send + 'static) -> SourceId {
        self.add_to(BusId::MASTER, source)
    }

    /// Add a source to a specific bus at unity gain; returns its handle. An
    /// unknown bus falls back to master.
    pub fn add_to(&mut self, bus: BusId, source: impl AudioSource + Send + 'static) -> SourceId {
        let bus = if (bus.0 as usize) < self.buses.len() {
            bus
        } else {
            BusId::MASTER
        };
        let id = self.next_id;
        self.next_id += 1;
        self.sources.push(MixedSource {
            id,
            source: Box::new(source),
            gain: 1.0,
            bus,
        });
        SourceId(id)
    }

    /// Create an input bus (sources → inserts → master, with optional sends).
    pub fn bus(&mut self, name: impl Into<String>) -> BusId {
        self.push_bus(name.into(), BusKind::Input, None)
    }

    /// Create an FX/return bus with an insert chain fed only by sends. Returns
    /// [`MixerError`] if the effects aren't streamable or the mixer has no rate.
    pub fn fx_bus(
        &mut self,
        name: impl Into<String>,
        effects: Vec<Node>,
    ) -> Result<BusId, MixerError> {
        let inserts = self.build_chain(&effects)?;
        Ok(self.push_bus(name.into(), BusKind::Fx, inserts))
    }

    fn push_bus(
        &mut self,
        name: String,
        kind: BusKind,
        inserts: Option<(EffectChain, EffectChain)>,
    ) -> BusId {
        // A bus's id is its index — buses are only ever pushed, never removed.
        let id = self.buses.len() as u32;
        self.buses.push(Bus {
            name,
            kind,
            gain: 1.0,
            to_master: 1.0,
            inserts,
            sends: Vec::new(),
        });
        BusId(id)
    }

    /// Look up a bus by name.
    pub fn bus_named(&self, name: &str) -> Option<BusId> {
        self.buses
            .iter()
            .position(|b| b.name == name)
            .map(|i| BusId(i as u32))
    }

    /// Set (or clear, with an empty list) a bus's insert chain. Works on any bus,
    /// including master. Returns [`MixerError::UnknownBus`] for a foreign/stale
    /// handle (it used to build the chain and silently discard it).
    pub fn set_bus_effects(&mut self, bus: BusId, effects: Vec<Node>) -> Result<(), MixerError> {
        let inserts = self.build_chain(&effects)?;
        match self.buses.get_mut(bus.0 as usize) {
            Some(b) => {
                b.inserts = inserts;
                Ok(())
            }
            None => Err(MixerError::UnknownBus),
        }
    }

    /// Set the master insert chain (a convenience for `set_bus_effects(MASTER, …)`).
    pub fn master_effects(&mut self, effects: Vec<Node>) -> Result<(), MixerError> {
        self.set_bus_effects(BusId::MASTER, effects)
    }

    /// Set a post-fader send from an input bus into an FX bus. A no-op unless
    /// `from` is an input bus and `to_fx` is an FX bus.
    pub fn set_send(&mut self, from: BusId, to_fx: BusId, level: f32) {
        let valid = matches!(
            self.buses.get(from.0 as usize).map(|b| b.kind),
            Some(BusKind::Input)
        ) && matches!(
            self.buses.get(to_fx.0 as usize).map(|b| b.kind),
            Some(BusKind::Fx)
        );
        if !valid {
            return;
        }
        let level = level.max(0.0);
        let bus = &mut self.buses[from.0 as usize];
        if let Some(s) = bus.sends.iter_mut().find(|s| s.0 == to_fx.0) {
            s.1 = level;
        } else {
            bus.sends.push((to_fx.0, level));
        }
    }

    /// Set a bus fader (0 = silent). Applies to the bus's dry output and its sends.
    pub fn set_bus_gain(&mut self, bus: BusId, gain: f32) {
        if let Some(b) = self.buses.get_mut(bus.0 as usize) {
            b.gain = gain.max(0.0);
        }
    }

    /// Set a bus's dry level into master (0 = send-only). No-op for master.
    pub fn set_bus_dry(&mut self, bus: BusId, level: f32) {
        if bus != BusId::MASTER
            && let Some(b) = self.buses.get_mut(bus.0 as usize)
        {
            b.to_master = level.max(0.0);
        }
    }

    /// Build a paired L/R insert chain from effect nodes (empty → `None`).
    fn build_chain(
        &self,
        effects: &[Node],
    ) -> Result<Option<(EffectChain, EffectChain)>, MixerError> {
        if effects.is_empty() {
            return Ok(None);
        }
        // The deprecated variant is still constructed until it is deleted at 2.0.
        #[allow(deprecated)]
        let sr = self.sample_rate.ok_or(MixerError::NoSampleRate)?;
        let build = || EffectChain::try_new(effects, sr, ENGINE_VERSION);
        let l = build().ok_or(MixerError::NotStreamable)?;
        let r = build().ok_or(MixerError::NotStreamable)?;
        Ok(Some((l, r)))
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

/// Grow `v` to at least `n` samples (never shrinks).
fn grow(v: &mut Vec<f32>, n: usize) {
    if v.len() < n {
        v.resize(n, 0.0);
    }
}

impl AudioSource for Mixer {
    fn fill(&mut self, out: &mut [f32]) -> usize {
        let frames = out.len() / 2;

        // Take the reusable planar buffers out so we can also borrow
        // self.sources / self.buses mutably during routing.
        let mut scratch = std::mem::take(&mut self.scratch);
        let mut master_l = std::mem::take(&mut self.master_l);
        let mut master_r = std::mem::take(&mut self.master_r);
        let mut bus_l = std::mem::take(&mut self.bus_l);
        let mut bus_r = std::mem::take(&mut self.bus_r);
        let mut fx_in = std::mem::take(&mut self.fx_in);

        grow(&mut scratch, frames * 2);
        grow(&mut master_l, frames);
        grow(&mut master_r, frames);
        grow(&mut bus_l, frames);
        grow(&mut bus_r, frames);
        if fx_in.len() < self.buses.len() {
            fx_in.resize_with(self.buses.len(), || (Vec::new(), Vec::new()));
        }
        for (l, r) in fx_in.iter_mut() {
            grow(l, frames);
            grow(r, frames);
        }

        master_l[..frames].fill(0.0);
        master_r[..frames].fill(0.0);
        for (l, r) in fx_in.iter_mut() {
            l[..frames].fill(0.0);
            r[..frames].fill(0.0);
        }

        let scr = &mut scratch[..frames * 2];

        // 1a. Master-direct sources sum straight into the master accumulator.
        for s in self.sources.iter_mut().filter(|s| s.bus == BusId::MASTER) {
            s.source.fill(scr);
            for f in 0..frames {
                master_l[f] += scr[f * 2] * s.gain;
                master_r[f] += scr[f * 2 + 1] * s.gain;
            }
        }

        // 1b. Input buses: sum sources → inserts → dry to master + post-fader sends.
        for bi in 1..self.buses.len() {
            if self.buses[bi].kind != BusKind::Input {
                continue;
            }
            bus_l[..frames].fill(0.0);
            bus_r[..frames].fill(0.0);
            let bus_id = bi as u32;
            for s in self.sources.iter_mut().filter(|s| s.bus.0 == bus_id) {
                s.source.fill(scr);
                for f in 0..frames {
                    bus_l[f] += scr[f * 2] * s.gain;
                    bus_r[f] += scr[f * 2 + 1] * s.gain;
                }
            }
            if let Some((cl, cr)) = &mut self.buses[bi].inserts {
                cl.process(&mut bus_l[..frames]);
                cr.process(&mut bus_r[..frames]);
            }
            let fader = self.buses[bi].gain;
            let dry = fader * self.buses[bi].to_master;
            for f in 0..frames {
                master_l[f] += bus_l[f] * dry;
                master_r[f] += bus_r[f] * dry;
            }
            for &(target, level) in &self.buses[bi].sends {
                let k = target as usize;
                if k < fx_in.len() {
                    let g = fader * level;
                    let (fl, fr) = &mut fx_in[k];
                    for f in 0..frames {
                        fl[f] += bus_l[f] * g;
                        fr[f] += bus_r[f] * g;
                    }
                }
            }
        }

        // 1c. Sources routed directly onto an FX bus are wet-only: sum them into
        // that bus's accumulator so its inserts process them like any send.
        // Without this a source added with `add_to(fx_bus, ..)` is never mixed and
        // its play head never advances.
        #[allow(clippy::needless_range_loop)]
        for bi in 1..self.buses.len() {
            if self.buses[bi].kind != BusKind::Fx {
                continue;
            }
            let bus_id = bi as u32;
            let (fl, fr) = &mut fx_in[bi];
            for s in self.sources.iter_mut().filter(|s| s.bus.0 == bus_id) {
                s.source.fill(scr);
                for f in 0..frames {
                    fl[f] += scr[f * 2] * s.gain;
                    fr[f] += scr[f * 2 + 1] * s.gain;
                }
            }
        }

        // 2. FX buses: run inserts on the accumulated sends, return to master.
        // `bi` indexes both self.buses and fx_in, so a range loop is clearest.
        #[allow(clippy::needless_range_loop)]
        for bi in 1..self.buses.len() {
            if self.buses[bi].kind != BusKind::Fx {
                continue;
            }
            let (fl, fr) = &mut fx_in[bi];
            if let Some((cl, cr)) = &mut self.buses[bi].inserts {
                cl.process(&mut fl[..frames]);
                cr.process(&mut fr[..frames]);
            }
            let ret = self.buses[bi].gain * self.buses[bi].to_master;
            for f in 0..frames {
                master_l[f] += fl[f] * ret;
                master_r[f] += fr[f] * ret;
            }
        }

        // 3. Master insert chain, then the master fader, then interleave out.
        if let Some((cl, cr)) = &mut self.buses[0].inserts {
            cl.process(&mut master_l[..frames]);
            cr.process(&mut master_r[..frames]);
        }
        let master_gain = self.buses[0].gain;
        for f in 0..frames {
            out[f * 2] = master_l[f] * master_gain;
            out[f * 2 + 1] = master_r[f] * master_gain;
        }

        self.scratch = scratch;
        self.master_l = master_l;
        self.master_r = master_r;
        self.bus_l = bus_l;
        self.bus_r = bus_r;
        self.fx_in = fx_in;
        frames
    }

    /// Rewind every source so a transport restart replays the mix from the top.
    /// Bus insert/send state (reverb tails, delay lines) is left ringing.
    fn reset(&mut self) {
        for s in self.sources.iter_mut() {
            s.source.reset();
        }
    }
}
