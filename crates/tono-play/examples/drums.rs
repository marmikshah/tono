//! Play a beat on the drum kit.
//!
//!     make play EXAMPLE=drums   (or `cargo run -p tono-play --example drums`)

use std::thread::sleep;
use std::time::Duration;

use tono_core::drumkit::DrumKit;
use tono_core::instrument::Note;
use tono_play::{Speaker, device_sample_rate};

fn main() -> anyhow::Result<()> {
    let sr = device_sample_rate()?;
    let speaker = Speaker::open(DrumKit::general_midi(sr))?;
    println!("boom-bap — Ctrl-C to stop");

    let step = Duration::from_millis(130);
    for _ in 0..8 {
        for i in 0..8u32 {
            speaker.control(|k| {
                k.note_on(Note(42), 0.5); // closed hat on every step
                if i == 0 || i == 4 {
                    k.note_on(Note(36), 1.0); // kick on 1 and 3
                }
                if i == 2 || i == 6 {
                    k.note_on(Note(38), 0.9); // snare on 2 and 4
                }
            });
            sleep(step);
        }
    }
    Ok(())
}
