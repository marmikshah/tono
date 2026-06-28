//! sonarium-desktop — the optional native studio.
//!
//! This binary is **not** part of the default build, the MCP server, or CI; it
//! is built only via `make desktop`. For now it exposes a native audio preview
//! CLI built on the real-time [`AudioEngine`]; the Tauri window (reusing the
//! node-patcher frontend) is wired on top of the same engine.

mod audio;

use audio::AudioEngine;
use sonarium_core::dsl::SoundDoc;

const HELP: &str = "sonarium-desktop — native real-time audio for sonarium.

USAGE:
    sonarium-desktop play FILE.json [SECS]   render a graph and play it through
                                             the default output device (SECS
                                             defaults to the document duration)";

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    match args.get(1).map(String::as_str) {
        Some("play") => play_cli(&args[2..]),
        _ => {
            println!("{HELP}");
            Ok(())
        }
    }
}

/// `sonarium-desktop play FILE [SECS]` — audition a graph natively via cpal.
fn play_cli(args: &[String]) -> anyhow::Result<()> {
    let path = args
        .first()
        .ok_or_else(|| anyhow::anyhow!("usage: sonarium-desktop play FILE.json [SECS]"))?;
    let doc: SoundDoc = serde_json::from_str(&std::fs::read_to_string(path)?)?;
    let secs = args
        .get(1)
        .and_then(|s| s.parse::<f32>().ok())
        .unwrap_or_else(|| doc.duration.max(0.5));

    let engine = AudioEngine::new(doc)?;
    println!(
        "playing for {secs:.2}s at {} Hz — Ctrl-C to stop",
        engine.device_sample_rate()
    );
    engine.play();
    std::thread::sleep(std::time::Duration::from_secs_f32(secs));
    Ok(())
}
