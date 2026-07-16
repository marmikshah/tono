//! instrument — a playable, pitched, polyphonic instrument built from a patch.
//!
//! Turns a [`Patch`](crate::patch::Patch) (a graph + named params) into something you *play* like a
//! GarageBand software instrument: pick the sound, then [`note_on`](Instrument::note_on) /
//! [`note_off`](Instrument::note_off) pitched notes with velocity. Each note is a
//! **voice** — the patch graph rendered at that note's pitch by the byte-identical
//! streaming renderer, shaped by a **gated** amplitude envelope (attack/decay/
//! sustain-while-held/release, unlike the graph's fixed-duration `Env`). Voices are
//! pooled with stealing, and the instrument mixes them to stereo.
//!
//! `Instrument` implements [`AudioSource`], so it drops straight onto a cpal /
//! AudioWorklet callback, or into a [`Mixer`](crate::runtime::Mixer) alongside SFX.

mod design;
mod envelope;
mod note;

#[cfg(test)]
mod tests;

pub use design::{InstrumentDesign, Modulation, PitchMap, PlayMode};
pub use envelope::EnvGen;
pub use note::{InstrumentError, Note};

use std::collections::BTreeMap;

use crate::dsl::{Node, SoundDoc, Value, note_to_hz};
use crate::runtime::AudioSource;
use crate::streaming::{EffectChain, StreamGraph};

/// Handle to one sounding voice (a single note-on). Stable until the voice is
/// culled.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct VoiceHandle(u64);

/// One detuned, panned copy in a (possibly unison) voice. The detune is baked
/// into the graph at build; `l`/`r` are its channel gains (already unison-
/// normalised so a stack isn't louder than a single voice).
struct UnisonCopy {
    graph: StreamGraph,
    l: f32,
    r: f32,
}

struct Voice {
    handle: u64,
    note: Note,
    /// The unison stack — one entry unless unison is on. All copies share the
    /// note pitch (so glide/bend move them together); each is detuned + panned.
    copies: Vec<UnisonCopy>,
    /// The frequency the graphs were baked at (the note the voice was built for).
    /// A live pitch scale of `target.freq() / built_hz` retunes to any other note
    /// without a rebuild — how mono glide moves between notes.
    built_hz: f32,
    env: EnvGen,
    gain: f32,
    /// Set once `note_off` has gated the envelope into release.
    releasing: bool,
    /// A note-off arrived while the sustain pedal was down — release on pedal-up.
    sustained: bool,
}

/// A polyphonic, pitched, gated instrument. Play it with
/// [`note_on`](Self::note_on) / [`note_off`](Self::note_off); it mixes its live
/// voices through [`AudioSource::fill`].
pub struct Instrument {
    sample_rate: u32,
    design: InstrumentDesign,
    /// Current parameter values (name → value); each new voice is built with these.
    values: BTreeMap<String, f32>,
    voices: Vec<Voice>,
    next_handle: u64,
    /// Sustain-pedal state: while down, note-offs are deferred until pedal-up.
    sustain: bool,
    /// Pitch-wheel bend as a frequency ratio (1.0 = centered), applied live to
    /// every sounding voice and any new one.
    bend: f32,
    /// Filter-cutoff (brightness) scale, 1.0 = as designed. Applied live to every
    /// voice's filters — a mod-wheel / CC74 brightness sweep without a rebuild.
    brightness: f32,
    /// Modulation LFO phases in `0..1`, advanced per control block. Phase
    /// accumulators (not an absolute sample clock) so precision never degrades:
    /// a `u64` clock cast to `f32` quantizes to whole control blocks after a
    /// few hours and the LFOs go steppy, then freeze.
    vib_phase: f32,
    flt_phase: f32,
    trem_phase: f32,
    /// Current tremolo gain (1.0 = no tremolo), updated at control rate.
    trem: f32,
    /// Notes physically held, oldest→newest — mono note priority. On a note-off
    /// the voice falls back to the last still-held note. Unused in poly mode.
    held: Vec<Note>,
    /// The shared master effect chain, one instance per stereo channel (identical
    /// coefficients, independent state) so a reverb/chorus reads as stereo. Both
    /// are one shared instance for the whole instrument — a tail outlives its note.
    master: Option<(EffectChain, EffectChain)>,
    /// Per-copy render scratch (mono).
    scratch: Vec<f32>,
    /// Per-voice amp-envelope scratch (one env, shared across its unison copies).
    env_buf: Vec<f32>,
    /// Summed-voices stereo bus, fed to the master chains.
    mix_l: Vec<f32>,
    mix_r: Vec<f32>,
}

/// Multiply every pitch-determining frequency (oscillator freqs, seq note
/// pitches) by `ratio`. Constant and note-name values are transposed; modulated
/// fundamentals are left as authored.
///
/// Looks like `vary::transpose_node` but is deliberately NOT unified with it:
/// the two walkers invert Modulated/RingMod/Modal handling (live play must
/// track the key on pitch-determining processors; humanize must not), and
/// unifying them would change rendered bytes.
fn transpose(node: &mut Node, ratio: f32) {
    fn scale(v: &mut Value, ratio: f32) {
        match v {
            Value::Const(c) => *c *= ratio,
            Value::Note(s) => *v = Value::Const(note_to_hz(s).unwrap_or(440.0) * ratio),
            Value::Modulated(_) => {}
        }
    }
    match node {
        Node::Sine { freq }
        | Node::Triangle { freq }
        | Node::Sawtooth { freq }
        | Node::Square { freq, .. }
        | Node::Fm { freq, .. }
        | Node::Super { freq, .. } => scale(freq, ratio),
        // Pitch-determining processors: the ring-mod carrier and the modal
        // body's resonant partials must track the note, or a bell/metallic patch
        // plays the same pitch for every key.
        Node::RingMod { freq } => scale(freq, ratio),
        Node::Modal { modes, .. } => {
            for m in modes.iter_mut() {
                m.freq *= ratio;
            }
        }
        Node::Seq { notes, .. } => {
            for note in notes.iter_mut() {
                scale(&mut note.pitch, ratio);
            }
        }
        Node::Mix { inputs } | Node::Mul { inputs } => {
            for i in inputs.iter_mut() {
                transpose(i, ratio);
            }
        }
        Node::Chain { stages } => {
            for s in stages.iter_mut() {
                transpose(s, ratio);
            }
        }
        Node::Tracks { tracks, master } => {
            for t in tracks.iter_mut() {
                transpose(&mut t.node, ratio);
            }
            for m in master.iter_mut() {
                transpose(m, ratio);
            }
        }
        _ => {}
    }
}

impl Instrument {
    /// Build an instrument from a design. Errors if the patch can't instantiate
    /// or its graph is outside the streamable subset — so every note is
    /// guaranteed to play in real time.
    pub fn new(design: InstrumentDesign, sample_rate: u32) -> Result<Self, InstrumentError> {
        let master = if design.master.is_empty() {
            None
        } else {
            let engine = design.patch.doc.effective_engine();
            let build = || EffectChain::try_new(&design.master, sample_rate, engine);
            let (l, r) = (
                build().ok_or(InstrumentError::NotStreamable)?,
                build().ok_or(InstrumentError::NotStreamable)?,
            );
            Some((l, r))
        };
        let values = design.patch.defaults();
        let inst = Instrument {
            sample_rate,
            design,
            values,
            voices: Vec::new(),
            next_handle: 1,
            sustain: false,
            bend: 1.0,
            brightness: 1.0,
            vib_phase: 0.0,
            flt_phase: 0.0,
            trem_phase: 0.0,
            trem: 1.0,
            held: Vec::new(),
            master,
            scratch: Vec::new(),
            env_buf: Vec::new(),
            mix_l: Vec::new(),
            mix_r: Vec::new(),
        };
        inst.build_result(Note::A4, 1.0, 1.0)?; // validate the reference voice
        Ok(inst)
    }

    /// Build the streamable graph for one note at the current parameter values.
    /// `detune` is a frequency multiplier baked into the graph (1.0 = none) — a
    /// unison copy bakes its slight detune here, so glide/bend (which ride the
    /// live pitch scale, keyed off the *nominal* note) preserve the spread.
    fn build_result(
        &self,
        note: Note,
        velocity: f32,
        detune: f32,
    ) -> Result<StreamGraph, InstrumentError> {
        let hz = note.freq() * detune;
        let mut values = self.values.clone();
        if let PitchMap::Param(name) = &self.design.pitch {
            values.insert(name.clone(), hz);
        }
        if let Some(vp) = &self.design.velocity_param {
            // Map velocity across the param's declared [min, max] (a musical
            // range), not the raw 0..1 — which would clamp to the minimum.
            if let Some(spec) = self.design.patch.params.iter().find(|p| &p.name == vp) {
                let (lo, hi) = (spec.min.min(spec.max), spec.min.max(spec.max));
                values.insert(vp.clone(), lo + velocity.clamp(0.0, 1.0) * (hi - lo));
            }
        }
        let mut doc: SoundDoc = self
            .design
            .patch
            .instantiate(&values)
            .map_err(|e| InstrumentError::BadPatch(e.to_string()))?;
        doc.sample_rate = self.sample_rate;
        if let PitchMap::Transpose { reference } = &self.design.pitch {
            transpose(&mut doc.root, hz / reference.freq());
        }
        StreamGraph::try_from_doc(&doc).ok_or(InstrumentError::NotStreamable)
    }

    /// Build the unison stack for `note`: `unison` detuned, panned, level-
    /// normalised copies (one copy, centered, when unison is off). `None` if the
    /// patch can't build.
    fn build_copies(&self, note: Note, velocity: f32) -> Option<Vec<UnisonCopy>> {
        let n = self.design.unison.max(1);
        let norm = 1.0 / (n as f32).sqrt(); // a stack shouldn't be louder than one
        let mut copies = Vec::with_capacity(n);
        for k in 0..n {
            // Spread copies symmetrically over [-1, 1] × the configured amounts.
            let spread = if n == 1 {
                0.0
            } else {
                (k as f32 / (n - 1) as f32 - 0.5) * 2.0
            };
            let detune = 2f32.powf(spread * self.design.detune_cents / 1200.0);
            let mut graph = self.build_result(note, velocity, detune).ok()?;
            if self.bend != 1.0 {
                graph.set_bend(self.bend);
            }
            if self.brightness != 1.0 {
                graph.set_cutoff(self.brightness); // catch a new note up to the knob
            }
            let pan = spread * self.design.unison_width;
            copies.push(UnisonCopy {
                graph,
                l: (1.0 - pan).min(1.0) * norm,
                r: (1.0 + pan).min(1.0) * norm,
            });
        }
        Some(copies)
    }

    /// Start a note; `velocity` in `[0, 1]` shapes its level. Returns the voice's
    /// handle. In poly mode each note is its own voice; if the pool is full the
    /// **quietest** voice is stolen (the least audible cut). In mono mode the one
    /// voice is retuned (gliding) to the new note. A patch made un-buildable by a
    /// bad param yields a silent voice rather than panicking — a control event
    /// never crashes the audio thread.
    pub fn note_on(&mut self, note: Note, velocity: f32) -> VoiceHandle {
        let velocity = velocity.clamp(0.0, 1.0);
        if let PlayMode::Mono { legato } = self.design.mode {
            return self.mono_note_on(note, velocity, legato);
        }
        let handle = self.next_handle;
        self.next_handle += 1;
        self.spawn_voice(handle, note, velocity);
        VoiceHandle(handle)
    }

    /// Build a fresh voice at `note` and add it to the pool, stealing the quietest
    /// if full. A no-op on an un-buildable patch (a bad param) — the caller still
    /// gets a handle, just a silent voice.
    fn spawn_voice(&mut self, handle: u64, note: Note, velocity: f32) {
        let Some(copies) = self.build_copies(note, velocity) else {
            return; // un-buildable patch ⇒ the caller keeps its handle, the voice is silent
        };
        let mut env = EnvGen::new(&self.design.amp, self.sample_rate);
        env.gate_on();
        // Steal by forcing the quietest sounding voice into a ~5 ms release —
        // never a mid-sample cut (an audible click on every steal). The pool
        // briefly holds the declicking voices on top of max_voices; a note
        // flood faster than the declick window falls back to hard removal so
        // the pool stays bounded.
        let sounding = self.voices.iter().filter(|v| !v.releasing).count();
        if sounding >= self.design.max_voices
            && let Some(victim) = self.quietest(|v| !v.releasing)
        {
            self.voices[victim].env.kill();
            self.voices[victim].releasing = true;
        }
        if self.voices.len() >= self.design.max_voices * 2
            && let Some(victim) = self.quietest(|_| true)
        {
            self.voices.remove(victim);
        }
        self.voices.push(Voice {
            handle,
            note,
            built_hz: note.freq(),
            copies,
            env,
            gain: velocity,
            releasing: false,
            sustained: false,
        });
    }

    /// The per-sample one-pole coefficient for the configured glide time (`1.0` =
    /// instant when glide is off).
    fn glide_coeff(&self) -> f32 {
        let secs = self.design.glide_secs;
        if secs <= 0.0 {
            1.0
        } else {
            1.0 - (-1.0 / (secs * self.sample_rate as f32)).exp()
        }
    }

    /// Mono note-on: retune the live voice (gliding) to `note`, or strike a fresh
    /// one if none is sounding. `legato` keeps the amp envelope running.
    fn mono_note_on(&mut self, note: Note, velocity: f32, legato: bool) -> VoiceHandle {
        self.held.retain(|&n| n != note);
        self.held.push(note);
        let coeff = self.glide_coeff();
        if let Some(v) = self.voices.iter_mut().find(|v| !v.releasing) {
            v.note = note;
            v.sustained = false;
            let scale = note.freq() / v.built_hz;
            for c in v.copies.iter_mut() {
                c.graph.glide_pitch(scale, coeff);
            }
            if !legato {
                v.env.gate_on(); // re-strike unless we're playing legato
                v.gain = velocity;
            }
            VoiceHandle(v.handle)
        } else {
            let handle = self.next_handle;
            self.next_handle += 1;
            self.spawn_voice(handle, note, velocity); // fresh attack — no glide
            VoiceHandle(handle)
        }
    }

    /// Mono note-off: fall back to the most-recent still-held note (gliding), or
    /// release the voice (deferred by the sustain pedal) when nothing is held.
    fn mono_note_off(&mut self, note: Note) -> bool {
        let before = self.held.len();
        self.held.retain(|&n| n != note);
        if self.held.len() == before {
            return false; // that note wasn't held
        }
        match self.held.last().copied() {
            Some(prev) => {
                let coeff = self.glide_coeff();
                if let Some(v) = self.voices.iter_mut().find(|v| !v.releasing) {
                    v.note = prev;
                    let scale = prev.freq() / v.built_hz;
                    for c in v.copies.iter_mut() {
                        c.graph.glide_pitch(scale, coeff);
                    }
                }
                true
            }
            None => {
                let sustain = self.sustain;
                for v in self.voices.iter_mut().filter(|v| !v.releasing) {
                    if sustain {
                        v.sustained = true;
                    } else {
                        v.env.gate_off();
                        v.releasing = true;
                    }
                }
                true
            }
        }
    }

    /// Index of the quietest voice among those matching `pick`.
    fn quietest(&self, pick: impl Fn(&Voice) -> bool) -> Option<usize> {
        self.voices
            .iter()
            .enumerate()
            .filter(|(_, v)| pick(v))
            .min_by(|(_, a), (_, b)| a.env.level().total_cmp(&b.env.level()))
            .map(|(i, _)| i)
    }

    /// Release the newest still-held voice of `note` (or defer it if the
    /// sustain pedal is down); returns whether a voice was released/deferred.
    /// MIDI note-off arrives by pitch, so this is the common path.
    pub fn note_off(&mut self, note: Note) -> bool {
        if matches!(self.design.mode, PlayMode::Mono { .. }) {
            return self.mono_note_off(note);
        }
        let sustain = self.sustain;
        match self
            .voices
            .iter_mut()
            .rev()
            .find(|v| v.note == note && !v.releasing && !v.sustained)
        {
            Some(v) if sustain => {
                v.sustained = true; // hold until pedal-up
                true
            }
            Some(v) => {
                v.env.gate_off();
                v.releasing = true;
                true
            }
            None => false,
        }
    }

    /// Set the sustain pedal. While down, note-offs are held; on release, every
    /// deferred voice enters its release. (MIDI CC64.)
    pub fn set_sustain(&mut self, down: bool) {
        self.sustain = down;
        if !down {
            for v in self.voices.iter_mut() {
                if v.sustained {
                    v.env.gate_off();
                    v.releasing = true;
                    v.sustained = false;
                }
            }
        }
    }

    /// Bend every sounding voice (and any struck later) by `semitones` — the
    /// pitch wheel. `0.0` is centered; a MIDI pitch wheel maps its ±8192 range to
    /// your chosen semitone span (commonly ±2). The bend is a pure repitch of the
    /// oscillators, applied live without rebuilding a voice.
    pub fn set_bend(&mut self, semitones: f32) {
        self.bend = 2f32.powf(semitones / 12.0);
        for v in self.voices.iter_mut() {
            for c in v.copies.iter_mut() {
                c.graph.set_bend(self.bend);
            }
        }
    }

    /// Sweep the filter cutoff of every sounding voice (and any struck later) —
    /// a live brightness control (`scale` multiplies each filter's cutoff, 1.0 =
    /// as designed). Recomputes coefficients in place, so a knob/CC74 sweep is
    /// click-free. Voices with no filter are simply unaffected.
    pub fn set_brightness(&mut self, scale: f32) {
        self.brightness = scale.max(0.01);
        for v in self.voices.iter_mut() {
            for c in v.copies.iter_mut() {
                c.graph.set_cutoff(self.brightness);
            }
        }
    }

    /// Release a specific voice by handle; returns whether it was found.
    pub fn release(&mut self, handle: VoiceHandle) -> bool {
        match self.voices.iter_mut().find(|v| v.handle == handle.0) {
            Some(v) => {
                v.env.gate_off();
                v.releasing = true;
                true
            }
            None => false,
        }
    }

    /// Release every held voice.
    pub fn all_notes_off(&mut self) {
        self.held.clear();
        for v in self.voices.iter_mut() {
            v.env.gate_off();
            v.releasing = true;
        }
    }

    /// Whether a handle still refers to a sounding voice.
    pub fn is_active(&self, handle: VoiceHandle) -> bool {
        self.voices.iter().any(|v| v.handle == handle.0)
    }

    /// The note a live voice is playing.
    pub fn voice_note(&self, handle: VoiceHandle) -> Option<Note> {
        self.voices
            .iter()
            .find(|v| v.handle == handle.0)
            .map(|v| v.note)
    }

    /// The pitch scale a voice is currently sounding at (1.0 = its built note),
    /// following an in-progress glide, excluding the pitch wheel. Useful for a
    /// live pitch readout.
    pub fn voice_pitch_scale(&self, handle: VoiceHandle) -> Option<f32> {
        self.voices
            .iter()
            .find(|v| v.handle == handle.0)
            .and_then(|v| v.copies.first())
            .map(|c| c.graph.pitch())
    }

    /// Set a named parameter for future notes. Returns whether it was accepted —
    /// rejected (and the previous value kept) if the name is unknown or the value
    /// would make the patch invalid, so the instrument can never reach an
    /// un-buildable state.
    pub fn set_param(&mut self, name: &str, value: f32) -> bool {
        if !self.design.patch.params.iter().any(|p| p.name == name) {
            return false;
        }
        let prev = self.values.insert(name.to_string(), value);
        if self.design.patch.instantiate(&self.values).is_ok() {
            true
        } else {
            match prev {
                Some(p) => self.values.insert(name.to_string(), p),
                None => self.values.remove(name),
            };
            false
        }
    }

    /// Number of live voices.
    pub fn active_voices(&self) -> usize {
        self.voices.len()
    }
}

impl Instrument {
    /// Update the modulation LFOs at their current phases, apply them to every
    /// voice (vibrato rides the bend channel, wobble the cutoff, tremolo the
    /// gain), then advance the phases by this `frames`-long control block.
    fn apply_modulation(&mut self, frames: usize) {
        let m = self.design.modulation;
        let tau = std::f32::consts::TAU;
        let vib = if m.vibrato_cents > 0.0 {
            2f32.powf((m.vibrato_cents / 1200.0) * (tau * self.vib_phase).sin())
        } else {
            1.0
        };
        let flt = if m.filter_octaves > 0.0 {
            2f32.powf(m.filter_octaves * (tau * self.flt_phase).sin())
        } else {
            1.0
        };
        self.trem = if m.tremolo_depth > 0.0 {
            1.0 - m.tremolo_depth * 0.5 * (1.0 - (tau * self.trem_phase).sin())
        } else {
            1.0
        };
        let step = frames as f32 / self.sample_rate as f32;
        self.vib_phase = (self.vib_phase + m.vibrato_rate * step).fract();
        self.flt_phase = (self.flt_phase + m.filter_rate * step).fract();
        self.trem_phase = (self.trem_phase + m.tremolo_rate * step).fract();
        let (bend, cutoff) = (self.bend * vib, self.brightness * flt);
        let wobble = m.filter_octaves > 0.0;
        for v in self.voices.iter_mut() {
            for c in v.copies.iter_mut() {
                c.graph.set_bend(bend);
                if wobble {
                    c.graph.set_cutoff(cutoff);
                }
            }
        }
    }

    /// Render one block at the current modulation state (the tremolo gain is
    /// baked into the amp envelope). Split out so `fill` can drive it at control
    /// rate when modulation is active.
    fn render_block(&mut self, out: &mut [f32]) {
        let frames = out.len() / 2;
        out.fill(0.0);
        if frames == 0 {
            return;
        }
        for buf in [
            &mut self.scratch,
            &mut self.env_buf,
            &mut self.mix_l,
            &mut self.mix_r,
        ] {
            if buf.len() < frames {
                buf.resize(frames, 0.0);
            }
        }
        let trem = self.trem;
        let copy = &mut self.scratch[..frames]; // per-copy render
        let env = &mut self.env_buf[..frames]; // per-voice envelope × gain
        let (mix_l, mix_r) = (&mut self.mix_l[..frames], &mut self.mix_r[..frames]);
        mix_l.fill(0.0);
        mix_r.fill(0.0);
        for v in self.voices.iter_mut() {
            // The amp envelope advances once per sample and is shared across the
            // voice's unison copies (they differ only in detune and pan).
            for e in env.iter_mut() {
                *e = v.env.tick() * v.gain * trem;
            }
            for c in v.copies.iter_mut() {
                c.graph.fill(copy);
                for f in 0..frames {
                    let s = copy[f] * env[f];
                    mix_l[f] += s * c.l;
                    mix_r[f] += s * c.r;
                }
            }
        }
        // One shared master per channel (a reverb tail is not multiplied per
        // voice); identical coefficients, independent state ⇒ a stereo image.
        if let Some((chain_l, chain_r)) = &mut self.master {
            chain_l.process(mix_l);
            chain_r.process(mix_r);
        }
        for f in 0..frames {
            out[f * 2] = mix_l[f];
            out[f * 2 + 1] = mix_r[f];
        }
        // Cull voices whose envelope has fully released — or a percussive voice
        // (sustain ≈ 0) that has decayed to silence but never got a note-off.
        self.voices.retain(|v| v.env.active() && !v.env.faded());
    }
}

impl AudioSource for Instrument {
    fn fill(&mut self, out: &mut [f32]) -> usize {
        let frames = out.len() / 2;
        // No modulation ⇒ render the whole block directly (byte-identical to a
        // pre-modulation instrument: trem stays 1.0).
        if !self.design.modulation.is_active() {
            self.render_block(out);
            return frames;
        }
        // Modulated ⇒ step the LFOs at control rate (64-frame sub-blocks) so
        // vibrato/wobble/tremolo move smoothly without per-sample coefficient cost.
        const CTRL: usize = 64;
        let mut done = 0;
        while done < frames {
            let n = CTRL.min(frames - done);
            self.apply_modulation(n);
            self.render_block(&mut out[done * 2..(done + n) * 2]);
            done += n;
        }
        frames
    }
}
