//! presets — a factory bank of ready-to-play instruments.
//!
//! Each [`Preset`] is a named [`InstrumentDesign`] authored over the same graph
//! vocabulary you'd write by hand, so it plays out of the box and is a worked
//! example of the instrument controls (mono/legato, glide, unison, velocity,
//! a shared master). Look them up by [`preset`] or iterate [`PRESETS`]:
//!
//! ```
//! use tono_core::instrument::Instrument;
//! use tono_core::presets;
//!
//! let design = presets::preset("warm_lead").unwrap();
//! let mut inst = Instrument::new(design, 48_000).unwrap();
//! inst.note_on(tono_core::instrument::Note::C4, 0.9);
//! ```

use serde::{Deserialize, Serialize};

use crate::dsl::Adsr;
use crate::instrument::{InstrumentDesign, PlayMode};
use crate::patch::Patch;

/// What a preset is for — a coarse grouping for browsing.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Category {
    /// Cutting melodic leads.
    Lead,
    /// Low-end foundations.
    Bass,
    /// Sustained atmospheric beds.
    Pad,
    /// Piano-like struck voices.
    Keys,
    /// Short plucked/percussive tones.
    Pluck,
}

/// One factory instrument: a stable name, a category, a one-line description,
/// and a builder for its [`InstrumentDesign`].
pub struct Preset {
    /// Stable lookup id (a slug like `"warm_lead"`).
    pub name: &'static str,
    /// Coarse grouping.
    pub category: Category,
    /// One-line description of the sound.
    pub description: &'static str,
    build: fn() -> InstrumentDesign,
}

impl Preset {
    /// Build a fresh [`InstrumentDesign`] for this preset.
    pub fn design(&self) -> InstrumentDesign {
        (self.build)()
    }
}

/// Look up a factory preset's design by name.
pub fn preset(name: &str) -> Option<InstrumentDesign> {
    PRESETS.iter().find(|p| p.name == name).map(Preset::design)
}

/// Every factory preset, in a stable order.
pub static PRESETS: &[Preset] = &[
    Preset {
        name: "warm_lead",
        category: Category::Lead,
        description: "Warm saw lead — mono, legato glide, velocity opens the filter.",
        build: warm_lead,
    },
    Preset {
        name: "square_lead",
        category: Category::Lead,
        description: "Bright square lead — mono glide, a touch of chiptune.",
        build: square_lead,
    },
    Preset {
        name: "supersaw_pad",
        category: Category::Pad,
        description: "Lush wide unison saw pad with a slow swell and reverb.",
        build: supersaw_pad,
    },
    Preset {
        name: "hollow_pad",
        category: Category::Pad,
        description: "Soft hollow triangle pad — wide unison, roomy.",
        build: hollow_pad,
    },
    Preset {
        name: "sub_bass",
        category: Category::Bass,
        description: "Deep sub bass — sine weight plus saw body, mono, snappy.",
        build: sub_bass,
    },
    Preset {
        name: "reese_bass",
        category: Category::Bass,
        description: "Detuned reese bass — mono legato, a bit of stereo width.",
        build: reese_bass,
    },
    Preset {
        name: "fm_tine",
        category: Category::Keys,
        description: "FM electric-piano tine — velocity brightens the bell.",
        build: fm_tine,
    },
    Preset {
        name: "pluck",
        category: Category::Pluck,
        description: "Short bright pluck — percussive, lightly detuned.",
        build: pluck,
    },
    Preset {
        name: "nylon",
        category: Category::Pluck,
        description: "Warm nylon-string pluck — a soft, rounded playable guitar.",
        build: nylon,
    },
    Preset {
        name: "vibrato_lead",
        category: Category::Lead,
        description: "Singing lead with vibrato — a saw that breathes as you hold it.",
        build: vibrato_lead,
    },
    Preset {
        name: "wobble_bass",
        category: Category::Bass,
        description: "Wobble bass — the filter sweeps under the note (dubstep-ish).",
        build: wobble_bass,
    },
];

/// Parse a factory patch. The JSON is a compile-time constant validated by the
/// `every_preset_builds_and_sounds` test, so a failure here is a build-time bug,
/// not a runtime-fallible path.
fn patch(json: &str) -> Patch {
    serde_json::from_str(json).expect("factory preset patch must be valid")
}

fn adsr(a: f32, d: f32, s: f32, r: f32) -> Adsr {
    Adsr {
        a,
        d,
        s,
        r,
        punch: 0.0,
    }
}

fn warm_lead() -> InstrumentDesign {
    InstrumentDesign::new(patch(
        r#"{ "doc": { "name":"warm_lead", "duration":1.0, "engine":2, "root": { "type":"chain", "stages": [
                { "type":"sawtooth", "freq":220 },
                { "type":"lowpass", "cutoff":2200, "q":0.9 } ] } },
             "params": [
                { "name":"pitch",  "paths":["root.stages[0].freq"],   "min":20,  "max":8000, "default":220 },
                { "name":"cutoff", "paths":["root.stages[1].cutoff"], "min":600, "max":7000, "default":2200 } ] }"#,
    ))
    .with_amp(adsr(0.01, 0.1, 0.8, 0.15))
    .with_mode(PlayMode::Mono { legato: true })
    .with_glide(0.06)
    .with_unison(2, 8.0, 0.3)
    .with_velocity_param("cutoff")
}

fn square_lead() -> InstrumentDesign {
    InstrumentDesign::new(patch(
        r#"{ "doc": { "name":"square_lead", "duration":1.0, "engine":2, "root": { "type":"chain", "stages": [
                { "type":"square", "freq":220, "duty":0.5 },
                { "type":"lowpass", "cutoff":4000, "q":0.7 } ] } },
             "params": [
                { "name":"pitch", "paths":["root.stages[0].freq"], "min":20, "max":8000, "default":220 } ] }"#,
    ))
    .with_amp(adsr(0.005, 0.05, 0.85, 0.08))
    .with_mode(PlayMode::Mono { legato: true })
    .with_glide(0.05)
}

fn supersaw_pad() -> InstrumentDesign {
    InstrumentDesign::new(patch(
        r#"{ "doc": { "name":"supersaw_pad", "duration":1.0, "engine":2, "root": { "type":"chain", "stages": [
                { "type":"sawtooth", "freq":220 },
                { "type":"lowpass", "cutoff":3000, "q":0.6 } ] } },
             "params": [
                { "name":"pitch", "paths":["root.stages[0].freq"], "min":20, "max":8000, "default":220 } ] }"#,
    ))
    .with_amp(adsr(0.6, 0.3, 0.8, 0.8))
    .with_unison(7, 30.0, 0.9)
    .with_master(vec![
        serde_json::from_str(r#"{ "type":"reverb", "room":0.7, "mix":0.35 }"#)
            .expect("factory master must be valid"),
    ])
}

fn hollow_pad() -> InstrumentDesign {
    InstrumentDesign::new(patch(
        r#"{ "doc": { "name":"hollow_pad", "duration":1.0, "engine":2, "root": { "type":"chain", "stages": [
                { "type":"triangle", "freq":220 },
                { "type":"lowpass", "cutoff":2500, "q":0.5 } ] } },
             "params": [
                { "name":"pitch", "paths":["root.stages[0].freq"], "min":20, "max":8000, "default":220 } ] }"#,
    ))
    .with_amp(adsr(0.4, 0.3, 0.75, 0.6))
    .with_unison(5, 18.0, 0.8)
    .with_master(vec![
        serde_json::from_str(r#"{ "type":"reverb", "room":0.6, "mix":0.3 }"#)
            .expect("factory master must be valid"),
    ])
}

fn sub_bass() -> InstrumentDesign {
    InstrumentDesign::new(patch(
        r#"{ "doc": { "name":"sub_bass", "duration":1.0, "engine":2, "root": { "type":"chain", "stages": [
                { "type":"mix", "inputs": [ { "type":"sine", "freq":55 }, { "type":"sawtooth", "freq":55 } ] },
                { "type":"lowpass", "cutoff":500, "q":0.8 } ] } },
             "params": [
                { "name":"pitch", "paths":["root.stages[0].inputs[0].freq","root.stages[0].inputs[1].freq"],
                  "min":20, "max":2000, "default":55 } ] }"#,
    ))
    .with_amp(adsr(0.005, 0.08, 0.9, 0.1))
    .with_mode(PlayMode::Mono { legato: false })
    .with_glide(0.02)
}

fn reese_bass() -> InstrumentDesign {
    InstrumentDesign::new(patch(
        r#"{ "doc": { "name":"reese_bass", "duration":1.0, "engine":2, "root": { "type":"chain", "stages": [
                { "type":"sawtooth", "freq":55 },
                { "type":"lowpass", "cutoff":700, "q":0.8 } ] } },
             "params": [
                { "name":"pitch", "paths":["root.stages[0].freq"], "min":20, "max":2000, "default":55 } ] }"#,
    ))
    .with_amp(adsr(0.005, 0.1, 0.85, 0.12))
    .with_mode(PlayMode::Mono { legato: true })
    .with_glide(0.03)
    .with_unison(3, 20.0, 0.4)
}

fn fm_tine() -> InstrumentDesign {
    InstrumentDesign::new(patch(
        r#"{ "doc": { "name":"fm_tine", "duration":1.0, "engine":2, "root": { "type":"chain", "stages": [
                { "type":"fm", "freq":220, "ratio":3.0, "index":3.5 },
                { "type":"lowpass", "cutoff":6000, "q":0.5 } ] } },
             "params": [
                { "name":"pitch",  "paths":["root.stages[0].freq"],  "min":20, "max":8000, "default":220 },
                { "name":"bright", "paths":["root.stages[0].index"], "min":1,  "max":8,    "default":3.5 } ] }"#,
    ))
    .with_amp(adsr(0.002, 0.5, 0.2, 0.4))
    .with_velocity_param("bright")
}

fn pluck() -> InstrumentDesign {
    InstrumentDesign::new(patch(
        r#"{ "doc": { "name":"pluck", "duration":1.0, "engine":2, "root": { "type":"chain", "stages": [
                { "type":"sawtooth", "freq":220 },
                { "type":"lowpass", "cutoff":3200, "q":0.8 } ] } },
             "params": [
                { "name":"pitch",  "paths":["root.stages[0].freq"],   "min":20,  "max":8000, "default":220 },
                { "name":"cutoff", "paths":["root.stages[1].cutoff"], "min":800, "max":7000, "default":3200 } ] }"#,
    ))
    .with_amp(adsr(0.001, 0.18, 0.0, 0.12))
    .with_unison(2, 6.0, 0.25)
    .with_velocity_param("cutoff")
}

fn nylon() -> InstrumentDesign {
    InstrumentDesign::new(patch(
        r#"{ "doc": { "name":"nylon", "duration":1.0, "engine":2, "root": { "type":"chain", "stages": [
                { "type":"mix", "inputs": [ { "type":"sawtooth", "freq":220 }, { "type":"triangle", "freq":220 } ] },
                { "type":"lowpass", "cutoff":2200, "q":0.7 } ] } },
             "params": [
                { "name":"pitch", "paths":["root.stages[0].inputs[0].freq","root.stages[0].inputs[1].freq"],
                  "min":20, "max":6000, "default":220 } ] }"#,
    ))
    .with_amp(adsr(0.003, 0.5, 0.0, 0.25))
    .with_unison(2, 5.0, 0.2)
}

fn vibrato_lead() -> InstrumentDesign {
    InstrumentDesign::new(patch(
        r#"{ "doc": { "name":"vibrato_lead", "duration":1.0, "engine":2, "root": { "type":"chain", "stages": [
                { "type":"sawtooth", "freq":220 },
                { "type":"lowpass", "cutoff":2600, "q":0.8 } ] } },
             "params": [
                { "name":"pitch", "paths":["root.stages[0].freq"], "min":20, "max":8000, "default":220 } ] }"#,
    ))
    .with_amp(adsr(0.02, 0.1, 0.85, 0.2))
    .with_mode(PlayMode::Mono { legato: true })
    .with_glide(0.05)
    .with_vibrato(5.5, 22.0)
}

fn wobble_bass() -> InstrumentDesign {
    InstrumentDesign::new(patch(
        r#"{ "doc": { "name":"wobble_bass", "duration":1.0, "engine":2, "root": { "type":"chain", "stages": [
                { "type":"sawtooth", "freq":55 },
                { "type":"lowpass", "cutoff":600, "q":0.9 } ] } },
             "params": [
                { "name":"pitch", "paths":["root.stages[0].freq"], "min":20, "max":2000, "default":55 } ] }"#,
    ))
    .with_amp(adsr(0.005, 0.1, 0.9, 0.15))
    .with_mode(PlayMode::Mono { legato: true })
    .with_unison(2, 16.0, 0.3)
    .with_wobble(3.5, 1.6)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::instrument::{Instrument, Note};
    use crate::runtime::AudioSource;

    fn peak(b: &[f32]) -> f32 {
        b.iter().fold(0.0f32, |m, &x| m.max(x.abs()))
    }

    #[test]
    fn every_preset_builds_and_sounds() {
        for p in PRESETS {
            let mut inst = Instrument::new(p.design(), 48_000)
                .unwrap_or_else(|e| panic!("preset {} failed to build: {e}", p.name));
            inst.note_on(Note::C4, 0.9);
            let mut out = vec![0.0f32; 4096 * 2];
            inst.fill(&mut out);
            assert!(peak(&out) > 0.0, "preset {} is silent", p.name);
        }
    }

    #[test]
    fn preset_lookup_by_name() {
        assert!(preset("warm_lead").is_some());
        assert!(preset("nope").is_none());
        assert_eq!(PRESETS.len(), 11);
    }

    #[test]
    fn presets_round_trip_through_serde() {
        for p in PRESETS {
            let json = serde_json::to_string(&p.design()).unwrap();
            let back: InstrumentDesign = serde_json::from_str(&json).unwrap();
            assert!(
                Instrument::new(back, 48_000).is_ok(),
                "preset {} recall failed",
                p.name
            );
        }
    }
}
