//! A live two-piece band: a drum kit and a piano in one Mixer, notes sent
//! dynamically from your code.
//!
//!     make play EXAMPLE=live_band   (or `cargo run -p tono-play --example live_band`)
//!
//! The shape every dynamic-instrument use has: a Mixer sums any set of
//! sources, `mixer.add` hands back a SourceId, and you drive that source live
//! through `get_mut::<T>(id)`. This is the exact pattern a game uses for
//! dynamic instruments and SFX.

use std::thread::sleep;
use std::time::Duration;

use tono_core::drumkit::DrumKit;
use tono_core::instrument::{Instrument, Note};
use tono_core::presets::preset;
use tono_core::runtime::Mixer;
use tono_play::{Speaker, device_sample_rate};

fn main() -> anyhow::Result<()> {
    let sr = device_sample_rate()?;
    let mut mixer = Mixer::new(sr);

    // Add the players; keep their ids to address them later.
    let drums = mixer.add(DrumKit::general_midi(sr));
    let piano = mixer.add(Instrument::new(preset("fm_tine").unwrap(), sr)?);

    // The player: owns the output stream and the audio callback; every live
    // change goes through `control` (the audio thread never blocks on it).
    let speaker = Speaker::open(mixer)?;
    println!("live band — a drum pattern with a chord per bar");

    let beat = Duration::from_millis(300); // ~100 bpm
    for bar in 0..4u32 {
        // Drums: one bar of kick/snare/hat.
        for step in 0..8u32 {
            speaker.control(|m| {
                let kit = m.get_mut::<DrumKit>(drums).unwrap();
                kit.note_on(Note(42), 0.5); // closed hat every eighth
                if step == 0 || step == 4 {
                    kit.note_on(Note(36), 1.0); // kick on 1 and 3
                }
                if step == 2 || step == 6 {
                    kit.note_on(Note(38), 0.9); // snare on 2 and 4
                }
            });
            sleep(beat / 2);
        }
        // Piano: a chord on the bar line, held across the bar, then released.
        let chord = [
            ["C4", "E4", "G4"],
            ["A3", "C4", "E4"],
            ["F3", "A3", "C4"],
            ["G3", "B3", "D4"],
        ][bar as usize];
        speaker.control(|m| {
            let keys = m.get_mut::<Instrument>(piano).unwrap();
            for (i, p) in chord.iter().enumerate() {
                keys.note_on(Note::parse(p).unwrap(), 0.8 - i as f32 * 0.1);
            }
        });
        sleep(beat * 3);
        speaker.control(|m| m.get_mut::<Instrument>(piano).unwrap().all_notes_off());
        sleep(beat);
    }
    Ok(())
}
