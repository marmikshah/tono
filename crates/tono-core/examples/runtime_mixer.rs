//! Headless demo of the tono runtime engine: load a patch, spawn several
//! independent instances, drive them by handle, and pull mixed audio — both
//! directly and across the wait-free control/audio `split()`.
//!
//! Run: `cargo run -p tono-core --example runtime_mixer`

use tono_core::dsl::SoundDoc;
use tono_core::runtime::{AudioSource, Engine, Tween};

fn peak(buf: &[f32]) -> f32 {
    buf.iter().fold(0.0f32, |m, &x| m.max(x.abs()))
}

fn blip() -> SoundDoc {
    serde_json::from_str(
        r#"{ "name":"blip", "duration":0.4, "root":{ "type":"sine", "freq":440 } }"#,
    )
    .expect("valid doc")
}

fn main() {
    let sr = 48_000;

    // --- Direct (single-threaded) use -------------------------------------
    let mut engine = Engine::new(sr);
    let patch = engine.load(&blip());

    // One patch → several independent instances, each controlled by handle.
    let a = engine.play_looping(patch);
    let b = engine.play_looping(patch);
    engine.set_gain(b, 0.5, Tween::ms(20.0, sr));
    engine.set_pan(a, -0.7, Tween::ms(20.0, sr));
    engine.set_pan(b, 0.7, Tween::ms(20.0, sr));

    let mut block = vec![0.0f32; 512 * 2];
    let mut p = 0.0f32;
    for _ in 0..20 {
        engine.fill(&mut block);
        p = p.max(peak(&block));
    }
    println!("direct mix: {} instances, peak {p:.3}", engine.active());

    // --- Split (control thread + audio thread) ----------------------------
    let mut engine = Engine::new(sr);
    let patch = engine.load(&blip());
    let (mut control, mut audio) = engine.split(2048);
    control.play_looping(patch); // Deref → Engine::play_looping

    control.pump(2048); // control thread: keep the ring fed
    let mut out = vec![0.0f32; 512 * 2];
    let frames = audio.fill(&mut out); // audio thread: drain in the callback
    println!("split mix: pulled {frames} frames, peak {:.3}", peak(&out));
}
