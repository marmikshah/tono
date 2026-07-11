//! The programmatic playground: build sounds and instruments in code and hear
//! them. Run:  make play   (or `cargo run -p tono-play --example playground`)

use std::thread::sleep;
use std::time::Duration;

use tono_core::dsl::{Adsr, SoundDoc};
use tono_core::instrument::{Instrument, InstrumentDesign, Note};
use tono_core::patch::Patch;
use tono_play::{Speaker, device_sample_rate, play_doc};

fn bleep() -> SoundDoc {
    serde_json::from_str(
        r#"{ "name":"bleep", "duration":0.25, "engine":2, "root": { "type":"mul", "inputs": [
            { "type":"sine", "freq":880 },
            { "type":"env", "a":0.002, "d":0.08, "s":0.0, "r":0.05 } ] } }"#,
    )
    .unwrap()
}

fn lead() -> Patch {
    serde_json::from_str(
        r#"{ "doc": { "name":"lead", "duration":1.0, "engine":2, "root": { "type":"chain", "stages": [
                { "type":"super", "wave":"sawtooth", "freq":220, "voices":5, "detune_cents":14 },
                { "type":"lowpass", "cutoff":2400, "q":0.9 } ] } },
             "params": [ { "name":"pitch", "paths":["root.stages[0].freq"], "min":20, "max":8000, "default":220 } ] }"#,
    )
    .unwrap()
}

fn main() -> anyhow::Result<()> {
    // 1) Make a sound and hear it — one call.
    println!("♪ a bleep…");
    play_doc(&bleep(), 0.5)?;

    // 2) Build an instrument and play a melody live through a held speaker.
    let sr = device_sample_rate()?;
    let inst = Instrument::new(
        InstrumentDesign::new(lead()).with_amp(Adsr {
            a: 0.01,
            d: 0.15,
            s: 0.6,
            r: 0.25,
            punch: 0.0,
        }),
        sr,
    )?;
    let speaker = Speaker::open(inst)?;
    println!("♪ a melody @ {} Hz…", speaker.sample_rate());
    for name in ["C4", "E4", "G4", "C5", "G4", "E4", "C4"] {
        let note = Note::parse(name).unwrap();
        speaker.control(|i| {
            i.note_on(note, 0.85);
        });
        sleep(Duration::from_millis(220));
        speaker.control(|i| {
            i.note_off(note);
        });
        sleep(Duration::from_millis(40));
    }
    sleep(Duration::from_millis(500)); // let the last release ring
    println!("done.");
    Ok(())
}
