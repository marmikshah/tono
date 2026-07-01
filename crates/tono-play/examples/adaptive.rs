//! Adaptive game music: stems fade in as the action heats up, plus a stinger.
//!
//!     cargo run -p tono-play --example adaptive
//!
//! Three looping stems on one intensity knob — drums always play, bass joins at
//! mid intensity, the lead swells in at high intensity — with a one-shot stinger
//! fired on an "event".

use std::thread::sleep;
use std::time::Duration;

use tono_core::adaptive::{AdaptiveMusic, LoopBuffer};
use tono_core::dsl::{Adsr, SeqWave, SoundDoc};
use tono_core::render;
use tono_core::song::{Song, note};
use tono_play::{Speaker, device_sample_rate};

/// Render a one-bar song and trim it to exactly one bar for a seamless loop.
fn stem(song: &Song, sr: u32) -> LoopBuffer {
    let mut doc = song.to_doc().expect("song compiles");
    doc.sample_rate = sr;
    let bar_secs = song.length_bars() as f32 * song.beats_per_bar as f32 * 60.0 / song.bpm;
    let n = (bar_secs * sr as f32) as usize;
    let p = render::render_product(&doc);
    let (mut l, mut r) = p.stereo.unwrap_or_else(|| (p.mono.clone(), p.mono));
    l.truncate(n);
    r.truncate(n);
    LoopBuffer::from_stereo(l, r)
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
    music.add_layer(stem(&drums, sr), 0.0); // always on
    music.add_layer(stem(&bass, sr), 0.34); // joins at mid intensity
    music.add_layer(stem(&lead, sr), 0.7); // swells in when it's intense
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
