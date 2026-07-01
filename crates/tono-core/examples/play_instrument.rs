//! Play a *designed instrument* like a mini GarageBand: build an instrument from
//! a patch (a supersaw lead), play a chord and a melody with velocity, release
//! notes, and host it in a mixer. Run:
//!   cargo run -p tono-core --example play_instrument

use tono_core::dsl::Adsr;
use tono_core::instrument::{Instrument, InstrumentDesign, Note};
use tono_core::patch::Patch;
use tono_core::runtime::{AudioSource, Mixer};

/// A supersaw lead with a `pitch` param bound to the oscillator — a designed,
/// playable instrument.
fn lead() -> Patch {
    serde_json::from_str(
        r#"{ "doc": { "name":"lead", "duration":1.0, "engine":2, "root": { "type":"chain", "stages": [
                { "type":"super", "wave":"sawtooth", "freq":220, "voices":5, "detune_cents":14 },
                { "type":"lowpass", "cutoff":2400, "q":0.9 } ] } },
             "params": [ { "name":"pitch", "paths":["root.stages[0].freq"], "min":20, "max":8000, "default":220 } ] }"#,
    )
    .expect("valid patch")
}

fn peak(b: &[f32]) -> f32 {
    b.iter().fold(0.0f32, |m, &x| m.max(x.abs()))
}

fn n(name: &str) -> Note {
    Note::parse(name).unwrap()
}

fn main() {
    let sr = 48_000;
    let design = InstrumentDesign::new(lead()).with_amp(Adsr {
        a: 0.01,
        d: 0.15,
        s: 0.6,
        r: 0.25,
        punch: 0.0,
    });
    let mut inst = Instrument::new(design, sr).expect("streamable instrument");

    // --- Play a C-major chord, hold, then release -------------------------
    inst.note_on(n("C4"), 0.9);
    inst.note_on(n("E4"), 0.8);
    inst.note_on(n("G4"), 0.7);
    println!("chord: {} voices", inst.active_voices());

    let mut block = vec![0.0f32; 512 * 2];
    let mut p = 0.0f32;
    for _ in 0..40 {
        inst.fill(&mut block);
        p = p.max(peak(&block));
    }
    inst.all_notes_off();
    for _ in 0..60 {
        inst.fill(&mut block);
    }
    println!(
        "after release: {} voices (chord peak {p:.3})",
        inst.active_voices()
    );

    // --- Host it in a mixer, and play a melody by reaching back in --------
    let mut mixer = Mixer::new();
    let lead_id = mixer.add(inst);
    mixer.set_gain(lead_id, 0.8);

    let melody = ["C4", "E4", "G4", "C5", "G4", "E4"];
    let mut apeak = 0.0f32;
    for name in melody {
        let voice = mixer.get_mut::<Instrument>(lead_id).unwrap();
        voice.note_on(n(name), 0.85);
        // ~120 ms of audio per note.
        for _ in 0..11 {
            mixer.fill(&mut block);
            apeak = apeak.max(peak(&block));
        }
        mixer
            .get_mut::<Instrument>(lead_id)
            .unwrap()
            .note_off(n(name));
    }
    println!("melody through the mixer: peak {apeak:.3}");
}
