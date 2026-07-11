//! Hear every factory instrument, played from a few lines of Rust.
//!
//!     make play EXAMPLE=presets   (or `cargo run -p tono-play --example presets`)
//!
//! Each preset is a ready-made [`InstrumentDesign`]; open a [`Speaker`] on it and
//! drive `note_on`/`note_off` live — the mono presets glide between notes, the
//! pads bloom, the pluck and tine ping.

use std::thread::sleep;
use std::time::Duration;

use tono_core::instrument::{Instrument, Note};
use tono_core::presets;
use tono_play::{Speaker, device_sample_rate};

fn main() -> anyhow::Result<()> {
    let sr = device_sample_rate()?;
    // A short riff (C major arpeggio up and back), played on each preset.
    let riff = [Note::C4, Note(64), Note(67), Note(72), Note(67), Note(64)];

    for p in presets::PRESETS {
        println!("♪ {:<12} — {}", p.name, p.description);
        let inst = Instrument::new(p.design(), sr)?;
        let speaker = Speaker::open(inst)?;
        for &n in &riff {
            speaker.control(|i| {
                i.note_on(n, 0.9);
            });
            sleep(Duration::from_millis(180));
            speaker.control(|i| {
                i.note_off(n);
            });
            sleep(Duration::from_millis(40));
        }
        sleep(Duration::from_millis(450)); // let the tail ring before the next
    }
    Ok(())
}
