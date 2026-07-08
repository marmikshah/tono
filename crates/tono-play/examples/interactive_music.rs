//! Interactive music: sections that switch on the bar, a tension knob, and an
//! on-beat stinger.
//!
//!     cargo run -p tono-play --example interactive_music
//!
//! An "explore" section plays; combat starts, so we `transition_to` a "battle"
//! section on the next bar (never a jarring mid-bar cut); intensity swells a lead
//! layer in; a boss stinger lands on the next bar.

use std::thread::sleep;
use std::time::Duration;

use tono_core::adaptive::{AdaptiveMusic, LoopBuffer, Quantize};
use tono_core::dsl::SoundDoc;
use tono_core::render;
use tono_play::{Speaker, device_sample_rate};

const BPM: f32 = 120.0;

/// A one-bar loop as a LoopBuffer (for a vertical intensity layer).
fn section(root: f32, sr: u32) -> LoopBuffer {
    let p = render::render_product(&section_doc(root, sr));
    let (l, r) = p.stereo.unwrap_or_else(|| (p.mono.clone(), p.mono));
    LoopBuffer::from_stereo(l, r)
}

fn midi(freq: f32) -> u32 {
    (69.0 + 12.0 * (freq / 440.0).log2()).round() as u32
}

fn main() -> anyhow::Result<()> {
    let sr = device_sample_rate()?;
    let mut music = AdaptiveMusic::new(sr);
    music.set_tempo(BPM, 4);

    // Two horizontal sections and a lead layer that swells with intensity.
    let explore = music.add_section("explore", &section_doc(220.0, sr));
    let battle = music.add_section("battle", &section_doc(261.63, sr));
    music.add_layer(section(330.0, sr), 0.6); // lead, joins at intensity ≥ 0.6
    let _ = explore;

    let stinger: SoundDoc = serde_json::from_str(&format!(
        r#"{{ "name":"boss", "duration":0.5, "sample_rate":{sr}, "engine":2,
             "root": {{ "type":"mul", "inputs": [
               {{ "type":"fm", "freq":110, "ratio":1.5, "index":8 }},
               {{ "type":"env", "a":0.001, "d":0.45, "s":0.0, "r":0.05 }} ] }} }}"#
    ))?;

    let speaker = Speaker::open(music)?;

    println!("explore…");
    sleep(Duration::from_secs(3));

    println!("combat! → battle section on the next bar, intensity up");
    speaker.control(|m| {
        m.transition_to(battle, Quantize::Bar);
        m.set_intensity(0.9); // the lead swells in (smooth cross-fade)
    });
    sleep(Duration::from_secs(4));

    println!("★ boss stinger on the next bar");
    speaker.control(|m| m.stinger_at(&stinger, Quantize::Bar));
    sleep(Duration::from_secs(4));
    Ok(())
}

/// A section as a fresh SoundDoc (add_section renders it into a LoopBuffer).
fn section_doc(root: f32, sr: u32) -> SoundDoc {
    let bar_secs = 4.0 * 60.0 / BPM;
    serde_json::from_str(&format!(
        r#"{{ "name":"sec", "duration":{bar_secs}, "sample_rate":{sr}, "engine":2,
             "root": {{ "type":"seq", "bpm":{BPM}, "steps_per_beat":2, "wave":"square",
                "env": {{ "a":0.002, "d":0.14, "s":0.0, "r":0.05 }},
                "notes": [
                  {{ "step":0, "len":1, "pitch":"midi:{n0}" }},
                  {{ "step":2, "len":1, "pitch":"midi:{n1}" }},
                  {{ "step":4, "len":1, "pitch":"midi:{n2}" }},
                  {{ "step":6, "len":1, "pitch":"midi:{n1}" }} ] }} }}"#,
        n0 = midi(root),
        n1 = midi(root) + 4,
        n2 = midi(root) + 7,
    ))
    .unwrap()
}
