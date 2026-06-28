//! WebAssembly bindings for the sonarium audio engine.
//!
//! One entry point — [`render`] — takes a `SoundDoc` graph as JSON and returns
//! the same things the MCP server hands an agent: stereo audio samples, the
//! analysis numbers, and the log-frequency spectrogram + waveform PNGs. It runs
//! the *identical* deterministic [`sonarium_core`] render path, so the browser
//! playground hears and sees exactly what an agent would. No filesystem, no
//! network: a graph in, audio + feedback out.
//!
//! The SoundFont `sampler` voice is unavailable here (it needs a file on disk);
//! every synthesis and SFX voice works.

use sonarium_core::{analysis, dsl::SoundDoc, dsl::Stereo, render};
use wasm_bindgen::prelude::*;

/// Install a panic hook that surfaces Rust panics in the browser console.
#[wasm_bindgen(start)]
pub fn start() {
    console_error_panic_hook::set_once();
}

/// The result of rendering one graph. Audio is exposed per channel as
/// `Float32Array`s and the two feedback images as `Uint8Array` PNG bytes;
/// `statsJson` is the serialized analysis (peak/RMS/LUFS/centroid/attack/…).
#[wasm_bindgen]
pub struct RenderResult {
    ok: bool,
    error: Option<String>,
    sample_rate: u32,
    channels: u32,
    duration: f32,
    left: Vec<f32>,
    right: Vec<f32>,
    spectrogram_png: Vec<u8>,
    waveform_png: Vec<u8>,
    stats_json: String,
}

#[wasm_bindgen]
impl RenderResult {
    /// Did the graph validate and render?
    #[wasm_bindgen(getter)]
    pub fn ok(&self) -> bool {
        self.ok
    }
    /// The validation / parse error message when `ok` is false.
    #[wasm_bindgen(getter)]
    pub fn error(&self) -> Option<String> {
        self.error.clone()
    }
    #[wasm_bindgen(getter, js_name = sampleRate)]
    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }
    /// 1 (mono) or 2 (stereo).
    #[wasm_bindgen(getter)]
    pub fn channels(&self) -> u32 {
        self.channels
    }
    /// Duration in seconds.
    #[wasm_bindgen(getter)]
    pub fn duration(&self) -> f32 {
        self.duration
    }
    /// Left/mid channel samples.
    #[wasm_bindgen(getter)]
    pub fn left(&self) -> Vec<f32> {
        self.left.clone()
    }
    /// Right channel samples (equals `left` for a mono render).
    #[wasm_bindgen(getter)]
    pub fn right(&self) -> Vec<f32> {
        self.right.clone()
    }
    /// The log-frequency spectrogram as PNG bytes.
    #[wasm_bindgen(getter, js_name = spectrogramPng)]
    pub fn spectrogram_png(&self) -> Vec<u8> {
        self.spectrogram_png.clone()
    }
    /// The waveform / amplitude-envelope image as PNG bytes.
    #[wasm_bindgen(getter, js_name = waveformPng)]
    pub fn waveform_png(&self) -> Vec<u8> {
        self.waveform_png.clone()
    }
    /// The full [`analysis::Analysis`] serialized to JSON (`{}` on failure).
    #[wasm_bindgen(getter, js_name = statsJson)]
    pub fn stats_json(&self) -> String {
        self.stats_json.clone()
    }
}

fn failed(error: String) -> RenderResult {
    RenderResult {
        ok: false,
        error: Some(error),
        sample_rate: 0,
        channels: 0,
        duration: 0.0,
        left: Vec::new(),
        right: Vec::new(),
        spectrogram_png: Vec::new(),
        waveform_png: Vec::new(),
        stats_json: "{}".to_string(),
    }
}

/// Render a `SoundDoc` JSON graph to audio + analysis + images. Validation and
/// parse failures come back as `{ ok: false, error }` rather than throwing, so
/// the playground can show the message live while the user edits.
#[wasm_bindgen]
pub fn render(graph_json: &str) -> RenderResult {
    let mut doc: SoundDoc = match serde_json::from_str(graph_json) {
        Ok(doc) => doc,
        Err(e) => return failed(format!("JSON parse error: {e}")),
    };
    doc.ensure_track_ids();
    if let Err(e) = doc.validate() {
        return failed(e);
    }

    let product = render::render_product(&doc);
    let is_stereo = product.stereo.is_some() || !matches!(doc.stereo, Stereo::Mono);
    let (left, right) = match product.stereo {
        Some((l, r)) => (l, r),
        None => render::stereoize(&product.mono, doc.stereo, doc.sample_rate),
    };
    let channels = if is_stereo { 2 } else { 1 };

    let stats = analysis::stats(&product.mono, doc.sample_rate);
    let stats_json = serde_json::to_string(&stats).unwrap_or_else(|_| "{}".to_string());

    RenderResult {
        ok: true,
        error: None,
        sample_rate: doc.sample_rate,
        channels,
        duration: stats.duration_secs,
        spectrogram_png: analysis::spectrogram_png(&product.mono).unwrap_or_default(),
        waveform_png: analysis::waveform_png(&product.mono).unwrap_or_default(),
        left,
        right,
        stats_json,
    }
}
