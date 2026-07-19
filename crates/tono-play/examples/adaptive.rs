//! Adaptive game music: stems fade in as the action heats up, plus a stinger.
//!
//!     cargo run -p tono-play --example adaptive
//!
//! Three looping stems on one intensity knob — drums always play, bass joins at
//! mid intensity, the lead swells in at high intensity — with a one-shot stinger
//! fired on an "event".

use std::thread::sleep;
use std::time::Duration;

use tono_core::adaptive::AdaptiveMusic;
use tono_core::dsl::{Adsr, SeqWave, SoundDoc};
use tono_core::song::{Song, note};
use tono_play::{Speaker, device_sample_rate};

/// Compile a one-bar song to a doc rendered at the device sample rate.
fn stem_doc(song: &Song, sr: u32) -> SoundDoc {
    let mut doc = song.to_doc().expect("song compiles");
    doc.sample_rate = sr;
    doc
}

fn one_bar(name: &str, wave: SeqWave, env: Adsr, notes: Vec<tono_core::dsl::SeqNote>) -> Song {
    let mut s = Song::new(name, 120.0);
    s.add_track("t", wave, env);
    s.add_pattern("p", 1, notes);
    s.arrange("t", "p", 0);
    s
}

fn main() -> anyhow::Result<()> {
    let sr = device_sample_rate()?;
    let short = Adsr {
        a: 0.001,
        d: 0.12,
        s: 0.0,
        r: 0.06,
        punch: 0.0,
    };
    let plucky = Adsr {
        a: 0.002,
        d: 0.18,
        s: 0.0,
        r: 0.1,
        punch: 0.0,
    };

    let drums = one_bar(
        "drums",
        SeqWave::Kit,
        short,
        vec![
            note(0, 2, "midi:36"),
            note(4, 2, "midi:42"),
            note(8, 2, "midi:38"),
            note(12, 2, "midi:42"),
        ],
    );
    let bass = one_bar(
        "bass",
        SeqWave::Bass,
        plucky,
        vec![note(0, 4, "C2"), note(8, 4, "G2")],
    );
    let lead = one_bar(
        "lead",
        SeqWave::Square,
        plucky,
        vec![
            note(0, 4, "C5"),
            note(4, 4, "E5"),
            note(8, 4, "G5"),
            note(12, 4, "E5"),
        ],
    );
    let stinger: SoundDoc = serde_json::from_str(
        r#"{ "name":"hit", "duration":0.4, "engine":2, "root": { "type":"mul", "inputs": [
            { "type":"fm", "freq":660, "ratio":2.5, "index":6 },
            { "type":"env", "a":0.001, "d":0.35, "s":0.0, "r":0.05 } ] } }"#,
    )?;

    let mut music = AdaptiveMusic::new(sr);
    // One phase-locked stem set: every stem lands on the same one-bar grid
    // (4 beats at 120 bpm), so the cross-fades stay sample-aligned.
    music.set_tempo(120.0, 4);
    music.add_stem_set(
        &[
            (&stem_doc(&drums, sr), 0.0), // always on
            (&stem_doc(&bass, sr), 0.34), // joins at mid intensity
            (&stem_doc(&lead, sr), 0.7),  // swells in when it's intense
        ],
        4.0,
    );
    let speaker = Speaker::open(music)?;

    // Ramp the action up, fire a stinger at the peak, then wind back down.
    for (label, level) in [("calm", 0.1), ("building", 0.5), ("combat!", 0.9)] {
        println!("intensity → {label}");
        speaker.control(|m| m.set_intensity(level));
        sleep(Duration::from_secs(4));
    }
    println!("★ stinger");
    speaker.control(|m| m.stinger(&stinger));
    sleep(Duration::from_secs(2));
    println!("intensity → calm");
    speaker.control(|m| m.set_intensity(0.1));
    sleep(Duration::from_secs(4));
    Ok(())
}
