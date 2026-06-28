//! sonarium-desktop — the optional native studio.
//!
//! Not part of the default build, the MCP server, or CI; built only via
//! `make desktop`. Two entry points on one engine:
//! - `sonarium-desktop` (no args) launches the Tauri window (the node-patcher
//!   frontend) with native real-time audio.
//! - `sonarium-desktop play FILE.json [SECS]` is a headless native preview.

mod audio;

use std::sync::Mutex;

use audio::AudioHandle;
use base64::Engine as _;
use serde::Serialize;
use sonarium_core::dsl::{Adsr, Shape, SoundDoc};
use sonarium_core::{analysis, render};
use tauri::State;

/// App state: the lazily-created audio engine handle (built on first render,
/// since it needs an output device).
#[derive(Default)]
struct Studio {
    engine: Mutex<Option<AudioHandle>>,
}

/// What `render_graph` hands the frontend: the same feedback the WASM build
/// returns, minus audio samples (audio plays natively through the engine).
#[derive(Serialize)]
struct RenderResult {
    ok: bool,
    error: Option<String>,
    sample_rate: u32,
    duration: f32,
    spectrogram_png: String,
    waveform_png: String,
    stats_json: String,
}

fn render_error(msg: String) -> RenderResult {
    RenderResult {
        ok: false,
        error: Some(msg),
        sample_rate: 0,
        duration: 0.0,
        spectrogram_png: String::new(),
        waveform_png: String::new(),
        stats_json: "{}".to_string(),
    }
}

/// Validate + render a graph: push it to the live audio engine and return the
/// spectrogram / waveform (base64 PNG) + analysis for the UI.
#[tauri::command]
fn render_graph(graph: String, studio: State<Studio>) -> RenderResult {
    let mut doc: SoundDoc = match serde_json::from_str(&graph) {
        Ok(d) => d,
        Err(e) => return render_error(format!("JSON parse error: {e}")),
    };
    doc.ensure_track_ids();
    if let Err(e) = doc.validate() {
        return render_error(e);
    }

    // Update (or create) the live audio engine. A missing audio device is
    // non-fatal — the visuals still render.
    if let Ok(mut slot) = studio.engine.lock() {
        match slot.as_ref() {
            Some(engine) => engine.set_doc(doc.clone()),
            None => *slot = audio::spawn(doc.clone()).ok(),
        }
    }

    let product = render::render_product(&doc);
    let stats = analysis::stats(&product.mono, doc.sample_rate);
    let b64 = |bytes: Vec<u8>| base64::engine::general_purpose::STANDARD.encode(bytes);
    RenderResult {
        ok: true,
        error: None,
        sample_rate: doc.sample_rate,
        duration: stats.duration_secs,
        spectrogram_png: b64(analysis::spectrogram_png(&product.mono).unwrap_or_default()),
        waveform_png: b64(analysis::waveform_png(&product.mono).unwrap_or_default()),
        stats_json: serde_json::to_string(&stats).unwrap_or_else(|_| "{}".to_string()),
    }
}

/// Transport control for the patch preview.
#[tauri::command]
fn transport(action: String, studio: State<Studio>) {
    if let Ok(slot) = studio.engine.lock()
        && let Some(engine) = slot.as_ref()
    {
        match action.as_str() {
            "play" => engine.play(),
            "stop" => engine.stop(),
            _ => {}
        }
    }
}

/// Set the live keyboard instrument (the patch's waveform + envelope).
#[tauri::command]
fn set_instrument(wave: String, a: f32, d: f32, s: f32, r: f32, duty: f32, studio: State<Studio>) {
    let shape = match wave.as_str() {
        "square" => Shape::Square,
        "triangle" => Shape::Triangle,
        "sawtooth" => Shape::Saw,
        _ => Shape::Sine,
    };
    let env = Adsr {
        a,
        d,
        s,
        r,
        punch: 0.0,
    };
    if let Ok(slot) = studio.engine.lock()
        && let Some(engine) = slot.as_ref()
    {
        engine.set_instrument(shape, env, duty);
    }
}

/// Strike a live note (`key` identifies it for `note_off`).
#[tauri::command]
fn note_on(key: u32, freq: f32, studio: State<Studio>) {
    if let Ok(slot) = studio.engine.lock()
        && let Some(engine) = slot.as_ref()
    {
        engine.note_on(key, freq);
    }
}

/// Release a live note.
#[tauri::command]
fn note_off(key: u32, studio: State<Studio>) {
    if let Ok(slot) = studio.engine.lock()
        && let Some(engine) = slot.as_ref()
    {
        engine.note_off(key);
    }
}

const HELP: &str = "sonarium-desktop — native sonarium studio.

USAGE:
    sonarium-desktop                         launch the studio window (real-time audio)
    sonarium-desktop play FILE.json [SECS]   headless: play a graph through the default device";

fn main() {
    let args: Vec<String> = std::env::args().collect();
    match args.get(1).map(String::as_str) {
        Some("play") => {
            if let Err(e) = play_cli(&args[2..]) {
                eprintln!("error: {e}");
                std::process::exit(1);
            }
        }
        Some("--help" | "-h") => println!("{HELP}"),
        _ => run_studio(),
    }
}

/// Launch the Tauri window with the node-patcher frontend.
fn run_studio() {
    tauri::Builder::default()
        .manage(Studio::default())
        .invoke_handler(tauri::generate_handler![
            render_graph,
            transport,
            set_instrument,
            note_on,
            note_off
        ])
        .run(tauri::generate_context!())
        .expect("failed to launch the sonarium studio window");
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

    let engine = audio::spawn(doc)?;
    println!(
        "playing for {secs:.2}s at {} Hz — Ctrl-C to stop",
        engine.device_sample_rate()
    );
    engine.play();
    std::thread::sleep(std::time::Duration::from_secs_f32(secs));
    Ok(())
}
