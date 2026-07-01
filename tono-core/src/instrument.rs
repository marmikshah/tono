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

use crate::dsl::{Adsr, Node, SoundDoc, Value, note_to_hz};
use crate::patch::Patch;
use crate::runtime::AudioSource;
use crate::streaming::StreamGraph;
use crate::voice::EnvGen;

/// A musical pitch as a MIDI note number (0–127). `A4` = 69 = 440 Hz.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, PartialOrd, Ord)]
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

/// How a note sets an instrument's pitch.
#[derive(Clone, Debug)]
pub enum PitchMap {
    /// Set this named patch parameter to the note's frequency (Hz). Precise — the
    /// patch author decides exactly what the pitch drives.
    Param(String),
    /// Transpose every source frequency in the graph by `note.freq() /
    /// reference.freq()`. Turns *any* sound into a playable instrument with no
    /// pitch param required.
    Transpose { reference: Note },
}

/// The recipe that makes a [`Patch`] playable.
#[derive(Clone, Debug)]
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
            patch,
        }
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
    scratch: Vec<f32>,
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
    /// Build an instrument from a design, validating that the patch renders and
    /// is streamable (so every note can play in real time). Errors on a bad patch
    /// or a graph outside the streamable subset.
    pub fn new(design: InstrumentDesign, sample_rate: u32) -> Result<Self, String> {
        let values = design.patch.defaults();
        let inst = Instrument {
            sample_rate,
            design,
            values,
            voices: Vec::new(),
            next_handle: 1,
            scratch: Vec::new(),
        };
        // Validate by building the reference voice's graph.
        inst.build_graph(Note::A4, 1.0)
            .ok_or_else(|| "instrument patch is not streamable at this sample rate".to_string())?;
        Ok(inst)
    }

    /// Build the streamable graph for one note at the current parameter values.
    fn build_graph(&self, note: Note, velocity: f32) -> Option<StreamGraph> {
        let mut values = self.values.clone();
        if let PitchMap::Param(name) = &self.design.pitch {
            values.insert(name.clone(), note.freq());
        }
        if let Some(vp) = &self.design.velocity_param {
            values.insert(vp.clone(), velocity);
        }
        let mut doc: SoundDoc = self.design.patch.instantiate(&values).ok()?;
        doc.sample_rate = self.sample_rate;
        if let PitchMap::Transpose { reference } = &self.design.pitch {
            transpose(&mut doc.root, note.freq() / reference.freq());
        }
        StreamGraph::try_from_doc(&doc)
    }

    /// Start a note. `velocity` in `[0, 1]` scales its level. Returns a handle to
    /// the new voice; the oldest voice is stolen if the pool is full.
    pub fn note_on(&mut self, note: Note, velocity: f32) -> VoiceHandle {
        let handle = self.next_handle;
        self.next_handle += 1;
        // The graph always builds (validated in `new`); if a param made it
        // un-streamable, fall back to the reference build so a note still sounds.
        let graph = self
            .build_graph(note, velocity)
            .or_else(|| self.build_graph(Note::A4, velocity))
            .expect("validated streamable in Instrument::new");
        let mut env = EnvGen::new(&self.design.amp, self.sample_rate);
        env.gate_on();
        if self.voices.len() >= self.design.max_voices.max(1) {
            // Steal the voice nearest the end of its life: a releasing one first,
            // else the oldest.
            let victim = self.voices.iter().position(|v| v.releasing).unwrap_or(0);
            self.voices.remove(victim);
        }
        self.voices.push(Voice {
            handle,
            note,
            graph,
            env,
            gain: velocity.clamp(0.0, 1.0),
            releasing: false,
        });
        VoiceHandle(handle)
    }

    /// Release the newest still-held voice of `note` (into its envelope's release).
    pub fn note_off(&mut self, note: Note) {
        if let Some(v) = self
            .voices
            .iter_mut()
            .rev()
            .find(|v| v.note == note && !v.releasing)
        {
            v.env.gate_off();
            v.releasing = true;
        }
    }

    /// Release a specific voice by handle.
    pub fn release(&mut self, handle: VoiceHandle) {
        if let Some(v) = self.voices.iter_mut().find(|v| v.handle == handle.0) {
            v.env.gate_off();
            v.releasing = true;
        }
    }

    /// Release every held voice.
    pub fn all_notes_off(&mut self) {
        for v in self.voices.iter_mut() {
            v.env.gate_off();
            v.releasing = true;
        }
    }

    /// Set a named parameter for **future** notes (existing voices keep the value
    /// they were struck with). Unknown names are ignored.
    pub fn set_param(&mut self, name: &str, value: f32) {
        if self.design.patch.params.iter().any(|p| p.name == name) {
            self.values.insert(name.to_string(), value);
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
        let mono = &mut self.scratch[..frames];
        for v in self.voices.iter_mut() {
            v.graph.fill(mono);
            for (f, &m) in mono.iter().enumerate() {
                let s = m * v.env.tick() * v.gain;
                out[f * 2] += s;
                out[f * 2 + 1] += s;
            }
        }
        // Cull voices whose envelope has fully released.
        self.voices.retain(|v| v.env.active());
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
    fn voice_stealing_caps_polyphony() {
        let design = InstrumentDesign {
            max_voices: 4,
            ..InstrumentDesign::new(saw_patch())
        };
        let mut inst = Instrument::new(design, 48_000).unwrap();
        for n in 60..70 {
            inst.note_on(Note(n), 0.8);
        }
        assert_eq!(inst.active_voices(), 4, "capped at max_voices");
    }
}
