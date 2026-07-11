//! Live DSP effects on mixer buses.
//!
//!     make play EXAMPLE=buses   (or `cargo run -p tono-play --example buses`)
//!
//! Two input buses — a "music" bus with an EQ + compressor insert, and a "sfx"
//! bus that sends into a shared reverb return — summing through a master
//! compressor. A lead plays on the music bus; plucks fire on the sfx bus and
//! ring out through the reverb.

use std::thread::sleep;
use std::time::Duration;

use tono_core::dsl::Node;
use tono_core::instrument::{Instrument, Note};
use tono_core::presets;
use tono_core::runtime::Mixer;
use tono_play::{Speaker, device_sample_rate};

fn node(json: &str) -> Node {
    serde_json::from_str(json).expect("valid effect node")
}

fn main() -> anyhow::Result<()> {
    let sr = device_sample_rate()?;
    let mut mixer = Mixer::new_at(sr);

    // A music bus: brighten with a peaking EQ, then glue with a compressor.
    let music = mixer.bus("music");
    mixer.set_bus_effects(
        music,
        vec![
            node(r#"{ "type":"peak", "cutoff":2500, "q":1.0, "gain_db":5 }"#),
            node(r#"{ "type":"compress", "threshold":-18, "ratio":3, "attack":0.005, "release":0.12, "makeup":3 }"#),
        ],
    )?;

    // A sfx bus that sends into a shared reverb return bus.
    let sfx = mixer.bus("sfx");
    let reverb = mixer.fx_bus(
        "reverb",
        vec![node(r#"{ "type":"reverb", "room":0.8, "mix":1.0 }"#)],
    )?;
    mixer.set_send(sfx, reverb, 0.9);
    mixer.set_bus_gain(reverb, 0.6);

    // A gentle master compressor over the whole sum.
    mixer.master_effects(vec![node(
        r#"{ "type":"compress", "threshold":-10, "ratio":2, "attack":0.01, "release":0.2, "makeup":1 }"#,
    )])?;

    // Instruments, each on its bus.
    let lead = Instrument::new(presets::preset("warm_lead").expect("preset"), sr)?;
    let lead_id = mixer.add_to(music, lead);
    let pluck = Instrument::new(presets::preset("pluck").expect("preset"), sr)?;
    let pluck_id = mixer.add_to(sfx, pluck);

    let speaker = Speaker::open(mixer)?;

    println!("lead on the music bus (EQ + compressor) · plucks on the sfx reverb send");
    let melody = ["C4", "E4", "G4", "B4", "A4", "G4", "E4", "C4"];
    for (i, n) in melody.iter().enumerate() {
        speaker.control(|m| {
            m.get_mut::<Instrument>(lead_id)
                .unwrap()
                .note_on(Note::parse(n).unwrap(), 0.8);
            if i % 2 == 0 {
                m.get_mut::<Instrument>(pluck_id)
                    .unwrap()
                    .note_on(Note::parse(n).unwrap().transpose(12), 0.9);
            }
        });
        sleep(Duration::from_millis(320));
        speaker.control(|m| {
            m.get_mut::<Instrument>(lead_id)
                .unwrap()
                .note_off(Note::parse(n).unwrap());
        });
    }
    // Let the reverb tail ring out.
    sleep(Duration::from_millis(1200));
    Ok(())
}
