//! Voice management: a flood of one-shots stays clean under a polyphony cap.
//!
//!     cargo run -p tono-play --example voices
//!
//! Fires far more SFX than the voice budget. Without a cap they'd pile up and
//! clip; `set_max_voices` steals the lowest-priority sounding voice instead, so a
//! busy arcade scene stays clean. The looping pad is CRITICAL — never stolen.

use std::thread::sleep;
use std::time::Duration;

use tono_core::dsl::SoundDoc;
use tono_core::runtime::{Engine, Priority};
use tono_play::{Speaker, device_sample_rate};

fn blip(freq: f32) -> SoundDoc {
    serde_json::from_str(&format!(
        r#"{{ "name":"blip", "duration":0.25, "engine":2, "root": {{ "type":"mul", "inputs": [
            {{ "type":"square", "freq":{freq} }},
            {{ "type":"env", "a":0.001, "d":0.18, "s":0.0, "r":0.05 }} ] }} }}"#
    ))
    .unwrap()
}

fn main() -> anyhow::Result<()> {
    let sr = device_sample_rate()?;
    let mut engine = Engine::new(sr);
    engine.set_max_voices(6); // a tight budget so stealing is audible

    let low = engine.load(&blip(880.0)); // zappy one-shots
    let pad = engine.load(&{
        let mut d: SoundDoc = serde_json::from_str(
            r#"{ "name":"pad", "duration":1.0, "engine":2, "root": { "type":"mul", "inputs": [
                { "type":"super", "freq":220, "voices":5, "detune":0.2 },
                { "type":"env", "a":0.2, "d":0.2, "s":0.8, "r":0.3 } ] } }"#,
        )
        .unwrap();
        d.sample_rate = sr;
        d
    });

    let (mut control, audio) = engine.split(2048);
    let _speaker = Speaker::open(audio)?;

    // A CRITICAL looping pad underneath — never stolen by the one-shot flood.
    control.play_looping_prioritized(pad, Priority::CRITICAL);

    println!("firing 60 one-shots into a 6-voice budget (pad stays put)…");
    for i in 0..60 {
        // A few are HIGH (accents) and win a voice over the LOW zaps.
        let prio = if i % 10 == 0 {
            Priority::HIGH
        } else {
            Priority::LOW
        };
        control.play_prioritized(low, prio);
        control.pump(1024);
        sleep(Duration::from_millis(60));
        control.pump(1024);
    }
    // Let the tails ring out.
    for _ in 0..40 {
        control.pump(1024);
        sleep(Duration::from_millis(20));
    }
    println!("done — never clipped, pad never dropped");
    Ok(())
}
