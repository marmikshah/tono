//! instrument — a playable, pitched, polyphonic instrument built from a patch.
//!
//! Turns a [`Patch`] (a graph + named params) into something you *play* like a
//! GarageBand software instrument: pick the sound, then [`note_on`](Instrument::note_on) /
//! [`note_off`](Instrument::note_off) pitched notes with velocity. Each note is a
//! **voice** — the patch graph rendered at that note's pitch by the byte-identical
//! streaming renderer, shaped by a **gated** amplitude envelope (attack/decay/
//! sustain-while-held/release, unlike the graph's fixed-duration `Env`). Voices are
//! pooled with stealing, and the instrument mixes them to stereo.
//!
//! `Instrument` implements [`AudioSource`], so it drops straight onto a cpal /
//! AudioWorklet callback, or into a [`Mixer`](crate::runtime::Mixer) alongside SFX.

use std::collections::BTreeMap;
use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

use crate::dsl::{Adsr, Node, SoundDoc, Value, note_to_hz};
use crate::patch::Patch;
use crate::runtime::AudioSource;
use crate::streaming::{EffectChain, StreamGraph};
use crate::voice::EnvGen;

/// Why an [`Instrument`] could not be built.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum InstrumentError {
    /// The patch's graph is outside the streaming subset, so it can't play in
    /// real time (e.g. a `tracks` root, a `normalize` stage, or a sampler seq).
    NotStreamable,
    /// The patch failed to instantiate at its defaults (a bad param path/value).
    BadPatch(String),
}

impl fmt::Display for InstrumentError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            InstrumentError::NotStreamable => {
                write!(
                    f,
                    "instrument patch is not streamable (can't play in real time)"
                )
            }
            InstrumentError::BadPatch(e) => write!(f, "instrument patch is invalid: {e}"),
        }
    }
}

impl std::error::Error for InstrumentError {}

/// A musical pitch as a MIDI note number (0–127). `A4` = 69 = 440 Hz.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Note(pub u8);

impl Note {
    /// Middle C.
    pub const C4: Note = Note(60);
    /// Concert A (440 Hz).
    pub const A4: Note = Note(69);

    /// The note's frequency in Hz (equal temperament, A4 = 440).
    pub fn freq(self) -> f32 {
        440.0 * 2f32.powf((self.0 as f32 - 69.0) / 12.0)
    }

    /// The MIDI note number.
    pub fn midi(self) -> u8 {
        self.0
    }

    /// Parse a note name (`"C4"`, `"F#3"`, `"Bb5"`) or `"midi:60"` into the
    /// nearest MIDI note.
    pub fn parse(s: &str) -> Option<Note> {
        let hz = note_to_hz(s)?;
        let midi = (69.0 + 12.0 * (hz / 440.0).log2()).round();
        if (0.0..=127.0).contains(&midi) {
            Some(Note(midi as u8))
        } else {
            None
        }
    }

    /// Shift by `semitones` (clamped to the MIDI range).
    pub fn transpose(self, semitones: i32) -> Note {
        Note((self.0 as i32 + semitones).clamp(0, 127) as u8)
    }
}

impl fmt::Display for Note {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        const NAMES: [&str; 12] = [
            "C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B",
        ];
        write!(
            f,
            "{}{}",
            NAMES[(self.0 % 12) as usize],
            self.0 as i32 / 12 - 1
        )
    }
}

impl FromStr for Note {
    type Err = ();
    fn from_str(s: &str) -> Result<Note, ()> {
        Note::parse(s).ok_or(())
    }
}

impl From<u8> for Note {
    fn from(midi: u8) -> Note {
        Note(midi)
    }
}

/// How a note sets an instrument's pitch.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum PitchMap {
    /// Set this named patch parameter to the note's frequency (Hz). Precise — the
    /// patch author decides exactly what the pitch drives.
    Param(String),
    /// Transpose every source frequency in the graph by `note.freq() /
    /// reference.freq()`. Turns *any* sound into a playable instrument with no
    /// pitch param required.
    Transpose { reference: Note },
}

/// The recipe that makes a [`Patch`] playable. Serializable, so an instrument is
/// a saveable/recallable preset (patch + envelope + pitch map + master).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct InstrumentDesign {
    /// The sound: a graph + its named params (authored as a sustaining voice).
    pub patch: Patch,
    /// The gated amplitude envelope applied per note (note-on → note-off).
    pub amp: Adsr,
    /// How a note maps to pitch.
    pub pitch: PitchMap,
    /// An optional param driven by velocity (e.g. filter cutoff for brightness).
    pub velocity_param: Option<String>,
    /// Maximum simultaneous voices before the oldest is stolen.
    pub max_voices: usize,
    /// A shared master effect chain applied to the summed voices (one reverb/delay
    /// for the whole instrument, not one per voice — so a tail outlives its note).
    /// Must be streamable processor nodes.
    pub master: Vec<Node>,
}

impl InstrumentDesign {
    /// A sensible default design for `patch`: a gentle amp envelope, 16 voices,
    /// and pitch by the param named `"pitch"` if the patch has one, else
    /// transpose from middle C.
    pub fn new(patch: Patch) -> Self {
        let has_pitch = patch.params.iter().any(|p| p.name == "pitch");
        InstrumentDesign {
            pitch: if has_pitch {
                PitchMap::Param("pitch".into())
            } else {
                PitchMap::Transpose {
                    reference: Note::C4,
                }
            },
            amp: Adsr {
                a: 0.005,
                d: 0.08,
                s: 0.7,
                r: 0.12,
                punch: 0.0,
            },
            velocity_param: None,
            max_voices: 16,
            master: Vec::new(),
            patch,
        }
    }

    /// Set the shared master effect chain (builder style) — e.g. one reverb for
    /// the whole instrument instead of per voice.
    pub fn with_master(mut self, master: Vec<Node>) -> Self {
        self.master = master;
        self
    }

    /// Set the amplitude envelope (builder style).
    pub fn with_amp(mut self, amp: Adsr) -> Self {
        self.amp = amp;
        self
    }

    /// Set the pitch mapping (builder style).
    pub fn with_pitch(mut self, pitch: PitchMap) -> Self {
        self.pitch = pitch;
        self
    }

    /// Drive a named param from note velocity, mapped across the param's declared
    /// `[min, max]` (e.g. a filter cutoff for velocity → brightness).
    pub fn with_velocity_param(mut self, name: impl Into<String>) -> Self {
        self.velocity_param = Some(name.into());
        self
    }

    /// Set the maximum simultaneous voices (at least 1).
    pub fn with_max_voices(mut self, max: usize) -> Self {
        self.max_voices = max.max(1);
        self
    }
}

/// Handle to one sounding voice (a single note-on). Stable until the voice is
/// culled.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct VoiceHandle(u64);

struct Voice {
    handle: u64,
    note: Note,
    graph: StreamGraph,
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
    /// The shared master effect chain (built from `design.master`).
    master: Option<EffectChain>,
    /// Per-voice render scratch (mono).
    scratch: Vec<f32>,
    /// Summed-voices bus (mono), fed to the master chain.
    mix: Vec<f32>,
}

/// Multiply every pitch-determining frequency (oscillator freqs, seq note
/// pitches) by `ratio`. Constant and note-name values are transposed; modulated
/// fundamentals are left as authored.
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
        | Node::Super { freq, .. } => scale(freq, ratio),
        Node::Square { freq, .. } => scale(freq, ratio),
        Node::Fm { freq, .. } => scale(freq, ratio),
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
            Some(
                EffectChain::try_new(&design.master, sample_rate, engine)
                    .ok_or(InstrumentError::NotStreamable)?,
            )
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
            master,
            scratch: Vec::new(),
            mix: Vec::new(),
        };
        inst.build_result(Note::A4, 1.0)?; // validate the reference voice
        Ok(inst)
    }

    /// Build the streamable graph for one note at the current parameter values.
    fn build_result(&self, note: Note, velocity: f32) -> Result<StreamGraph, InstrumentError> {
        let mut values = self.values.clone();
        if let PitchMap::Param(name) = &self.design.pitch {
            values.insert(name.clone(), note.freq());
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
            .map_err(InstrumentError::BadPatch)?;
        doc.sample_rate = self.sample_rate;
        if let PitchMap::Transpose { reference } = &self.design.pitch {
            transpose(&mut doc.root, note.freq() / reference.freq());
        }
        StreamGraph::try_from_doc(&doc).ok_or(InstrumentError::NotStreamable)
    }

    /// Start a note; `velocity` in `[0, 1]` shapes its level. Returns the voice's
    /// handle. If the pool is full the **quietest** voice is stolen (the least
    /// audible cut). A patch made un-buildable by a bad param yields a silent
    /// voice rather than panicking — a control event never crashes the audio thread.
    pub fn note_on(&mut self, note: Note, velocity: f32) -> VoiceHandle {
        let handle = self.next_handle;
        self.next_handle += 1;
        let velocity = velocity.clamp(0.0, 1.0);
        if let Ok(mut graph) = self.build_result(note, velocity) {
            if self.bend != 1.0 {
                graph.set_bend(self.bend); // catch a newly struck note up to the wheel
            }
            let mut env = EnvGen::new(&self.design.amp, self.sample_rate);
            env.gate_on();
            if self.voices.len() >= self.design.max_voices
                && let Some(victim) = self.quietest()
            {
                self.voices.remove(victim);
            }
            self.voices.push(Voice {
                handle,
                note,
                graph,
                env,
                gain: velocity,
                releasing: false,
                sustained: false,
            });
        }
        VoiceHandle(handle)
    }

    fn quietest(&self) -> Option<usize> {
        self.voices
            .iter()
            .enumerate()
            .min_by(|(_, a), (_, b)| a.env.level().total_cmp(&b.env.level()))
            .map(|(i, _)| i)
    }

    /// Release the newest still-held voice of `note` (or defer it if the sustain
    /// pedal is down); returns how many were released/deferred (0 or 1). MIDI
    /// note-off arrives by pitch, so this is the common path.
    pub fn note_off(&mut self, note: Note) -> usize {
        let sustain = self.sustain;
        match self
            .voices
            .iter_mut()
            .rev()
            .find(|v| v.note == note && !v.releasing && !v.sustained)
        {
            Some(v) if sustain => {
                v.sustained = true; // hold until pedal-up
                1
            }
            Some(v) => {
                v.env.gate_off();
                v.releasing = true;
                1
            }
            None => 0,
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
            v.graph.set_bend(self.bend);
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

impl AudioSource for Instrument {
    fn fill(&mut self, out: &mut [f32]) -> usize {
        let frames = out.len() / 2;
        out.fill(0.0);
        if frames == 0 {
            return 0;
        }
        if self.scratch.len() < frames {
            self.scratch.resize(frames, 0.0);
        }
        if self.mix.len() < frames {
            self.mix.resize(frames, 0.0);
        }
        let voice = &mut self.scratch[..frames]; // per-voice render
        let mix = &mut self.mix[..frames]; // summed bus
        mix.fill(0.0);
        for v in self.voices.iter_mut() {
            v.graph.fill(voice);
            for (m, &s) in mix.iter_mut().zip(voice.iter()) {
                *m += s * v.env.tick() * v.gain;
            }
        }
        // One shared master chain (a reverb tail is not multiplied per voice).
        if let Some(chain) = &mut self.master {
            chain.process(mix);
        }
        for (f, &m) in mix.iter().enumerate() {
            out[f * 2] = m;
            out[f * 2 + 1] = m;
        }
        // Cull voices whose envelope has fully released — or a percussive voice
        // (sustain ≈ 0) that has decayed to silence but never got a note-off.
        self.voices.retain(|v| v.env.active() && !v.env.faded());
        frames
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn saw_patch() -> Patch {
        // A sustaining subtractive voice with a `pitch` param on the oscillator.
        serde_json::from_str(
            r#"{ "doc": { "name":"lead", "duration":1.0, "engine":2, "root": { "type":"chain", "stages": [
                    { "type":"sawtooth", "freq":220 },
                    { "type":"lowpass", "cutoff":1800, "q":0.8 } ] } },
                 "params": [ { "name":"pitch", "paths":["root.stages[0].freq"], "min":20, "max":8000, "default":220 } ] }"#,
        )
        .unwrap()
    }

    fn peak(b: &[f32]) -> f32 {
        b.iter().fold(0.0f32, |m, &x| m.max(x.abs()))
    }

    fn bits(b: &[f32]) -> Vec<u32> {
        b.iter().map(|x| x.to_bits()).collect()
    }

    #[test]
    fn note_maths() {
        assert!((Note::A4.freq() - 440.0).abs() < 1e-3);
        assert!((Note::C4.freq() - 261.6256).abs() < 1e-2);
        assert_eq!(Note::parse("A4"), Some(Note::A4));
        assert_eq!(Note::parse("midi:60"), Some(Note::C4));
        assert_eq!(Note::C4.transpose(12), Note(72));
    }

    #[test]
    fn plays_polyphonic_pitched_notes() {
        let mut inst = Instrument::new(InstrumentDesign::new(saw_patch()), 48_000).unwrap();
        // A three-note chord.
        inst.note_on(Note::C4, 0.9);
        inst.note_on(Note(64), 0.8); // E4
        inst.note_on(Note(67), 0.7); // G4
        assert_eq!(inst.active_voices(), 3);

        let mut out = vec![0.0f32; 512 * 2];
        assert_eq!(inst.fill(&mut out), 512);
        assert!(peak(&out) > 0.0, "chord makes sound");
        // Mono duplicated to stereo.
        assert!((0..512).all(|f| out[f * 2] == out[f * 2 + 1]));
    }

    #[test]
    fn note_off_releases_then_culls() {
        let amp = Adsr {
            a: 0.001,
            d: 0.001,
            s: 0.8,
            r: 0.01,
            punch: 0.0,
        };
        let design = InstrumentDesign::new(saw_patch()).with_amp(amp);
        let mut inst = Instrument::new(design, 48_000).unwrap();
        inst.note_on(Note::A4, 1.0);
        assert_eq!(inst.active_voices(), 1);
        // Let attack/decay settle, then release.
        let mut out = vec![0.0f32; 256 * 2];
        inst.fill(&mut out);
        inst.note_off(Note::A4);
        // Serve well past the 10 ms release (480 frames) so it culls.
        let mut tail = vec![0.0f32; 2048 * 2];
        inst.fill(&mut tail);
        assert_eq!(inst.active_voices(), 0, "released voice is culled");
    }

    #[test]
    fn transpose_makes_any_sound_playable() {
        // A bare saw with no pitch param — playable via transposition.
        let patch: Patch = serde_json::from_str(
            r#"{ "doc": { "name":"buzz", "duration":1.0, "engine":2, "root": { "type":"sawtooth", "freq":220 } } }"#,
        )
        .unwrap();
        let design = InstrumentDesign::new(patch); // no "pitch" param ⇒ Transpose
        assert!(matches!(design.pitch, PitchMap::Transpose { .. }));
        let mut inst = Instrument::new(design, 48_000).unwrap();
        inst.note_on(Note::C4, 1.0);
        inst.note_on(Note(72), 1.0); // an octave up
        let mut out = vec![0.0f32; 256 * 2];
        inst.fill(&mut out);
        assert!(peak(&out) > 0.0);
    }

    #[test]
    fn pitch_bend_repitches_live() {
        // Bending A4 up an octave is a pure repitch, so it is bit-for-bit A5
        // struck plain (same oscillator phase increment, same baked filter).
        let mut a = Instrument::new(InstrumentDesign::new(saw_patch()), 48_000).unwrap();
        a.note_on(Note::A4, 1.0);
        a.set_bend(12.0);
        let mut b = Instrument::new(InstrumentDesign::new(saw_patch()), 48_000).unwrap();
        b.note_on(Note(81), 1.0); // A5
        let (mut oa, mut ob) = (vec![0.0f32; 2048], vec![0.0f32; 2048]);
        a.fill(&mut oa);
        b.fill(&mut ob);
        assert_eq!(bits(&oa), bits(&ob), "A4 + octave bend == A5");
    }

    #[test]
    fn centered_bend_is_a_no_op() {
        let mut bent = Instrument::new(InstrumentDesign::new(saw_patch()), 48_000).unwrap();
        bent.note_on(Note::C4, 0.8);
        bent.set_bend(0.0); // dead center
        let mut plain = Instrument::new(InstrumentDesign::new(saw_patch()), 48_000).unwrap();
        plain.note_on(Note::C4, 0.8);
        let (mut ob, mut op) = (vec![0.0f32; 1024], vec![0.0f32; 1024]);
        bent.fill(&mut ob);
        plain.fill(&mut op);
        assert_eq!(bits(&ob), bits(&op), "a centered wheel changes nothing");
    }

    #[test]
    fn voice_stealing_caps_polyphony() {
        let design = InstrumentDesign::new(saw_patch()).with_max_voices(4);
        let mut inst = Instrument::new(design, 48_000).unwrap();
        for n in 60..70 {
            inst.note_on(Note(n), 0.8);
        }
        assert_eq!(inst.active_voices(), 4, "capped at max_voices");
    }

    #[test]
    fn note_names_round_trip_and_convert() {
        assert_eq!(Note::A4.to_string(), "A4");
        assert_eq!(Note::C4.to_string(), "C4");
        assert_eq!("F#3".parse::<Note>().unwrap().to_string(), "F#3");
        assert_eq!(Note::from(60u8), Note::C4);
        assert!("nonsense".parse::<Note>().is_err());
    }

    #[test]
    fn transpose_scales_all_pitched_nodes() {
        // Regression: modal mode freqs and the ring-mod carrier must transpose too.
        let mut doc: SoundDoc = serde_json::from_str(
            r#"{ "name":"b", "duration":0.1, "root": { "type":"chain", "stages": [
                { "type":"sawtooth", "freq":100 },
                { "type":"ringmod", "freq":200 },
                { "type":"modal", "modes":[{ "freq":300, "decay":0.3, "gain":1.0 }], "mix":0.5 } ] } }"#,
        )
        .unwrap();
        transpose(&mut doc.root, 2.0);
        let v = serde_json::to_value(&doc).unwrap();
        let stages = &v["root"]["stages"];
        assert_eq!(stages[0]["freq"], 200.0);
        assert_eq!(stages[1]["freq"], 400.0);
        assert_eq!(stages[2]["modes"][0]["freq"], 600.0);
    }

    #[test]
    fn handles_and_note_off_count() {
        let mut inst = Instrument::new(InstrumentDesign::new(saw_patch()), 48_000).unwrap();
        let h = inst.note_on(Note::C4, 0.9);
        assert!(inst.is_active(h));
        assert_eq!(inst.voice_note(h), Some(Note::C4));
        assert_eq!(inst.note_off(Note::C4), 1);
        assert_eq!(inst.note_off(Note::C4), 0, "already releasing");
        assert_eq!(inst.note_off(Note(80)), 0, "no such note");
    }

    #[test]
    fn set_param_validates_and_note_on_never_panics() {
        let mut inst = Instrument::new(InstrumentDesign::new(saw_patch()), 48_000).unwrap();
        assert!(!inst.set_param("nope", 1.0), "unknown param rejected");
        assert!(inst.set_param("pitch", 300.0), "valid value accepted");
        // note_on must never panic on a control event.
        inst.note_on(Note::A4, 1.0);
        assert_eq!(inst.active_voices(), 1);
    }

    #[test]
    fn percussive_voice_culls_without_note_off() {
        // sustain = 0 ⇒ a one-shot fired via note_on only must not leak voices.
        let amp = Adsr {
            a: 0.001,
            d: 0.02,
            s: 0.0,
            r: 0.05,
            punch: 0.0,
        };
        let design = InstrumentDesign::new(saw_patch()).with_amp(amp);
        let mut inst = Instrument::new(design, 48_000).unwrap();
        inst.note_on(Note::C4, 1.0);
        let mut out = vec![0.0f32; 2048 * 2];
        inst.fill(&mut out); // past the ~20 ms decay to silence
        assert_eq!(
            inst.active_voices(),
            0,
            "percussive one-shot reclaims its voice"
        );
    }

    #[test]
    fn master_reverb_tail_outlives_the_voice() {
        let reverb: Node =
            serde_json::from_str(r#"{ "type":"reverb", "room":0.8, "mix":0.6 }"#).unwrap();
        let design = InstrumentDesign::new(saw_patch()).with_master(vec![reverb]);
        let mut inst = Instrument::new(design, 48_000).unwrap();
        inst.note_on(Note::A4, 1.0);
        let mut out = vec![0.0f32; 256 * 2];
        inst.fill(&mut out);
        inst.all_notes_off();
        for _ in 0..40 {
            inst.fill(&mut out); // let the ~120 ms release finish and cull the voice
        }
        assert_eq!(inst.active_voices(), 0);
        // The one shared master reverb still rings after the voice is gone.
        let mut tail = vec![0.0f32; 256 * 2];
        inst.fill(&mut tail);
        assert!(
            peak(&tail) > 0.0,
            "shared reverb tail continues past the note"
        );
    }

    #[test]
    fn sustain_pedal_defers_release() {
        let amp = Adsr {
            a: 0.001,
            d: 0.001,
            s: 0.8,
            r: 0.01,
            punch: 0.0,
        };
        let design = InstrumentDesign::new(saw_patch()).with_amp(amp);
        let mut inst = Instrument::new(design, 48_000).unwrap();
        inst.set_sustain(true);
        inst.note_on(Note::A4, 1.0);
        let mut out = vec![0.0f32; 256 * 2];
        inst.fill(&mut out);
        inst.note_off(Note::A4); // deferred by the pedal
        for _ in 0..40 {
            inst.fill(&mut out);
        }
        assert_eq!(inst.active_voices(), 1, "held by the sustain pedal");
        inst.set_sustain(false); // pedal up → release
        for _ in 0..40 {
            inst.fill(&mut out);
        }
        assert_eq!(inst.active_voices(), 0, "released on pedal-up");
    }

    #[test]
    fn design_round_trips_through_serde() {
        let design = InstrumentDesign::new(saw_patch());
        let json = serde_json::to_string(&design).unwrap();
        let recalled: InstrumentDesign = serde_json::from_str(&json).unwrap();
        assert!(
            Instrument::new(recalled, 48_000).is_ok(),
            "preset recall works"
        );
    }
}
