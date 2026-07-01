//! Python bindings for tono — make sounds and play them from Python.
//!
//! ```python
//! import tono, json
//! doc = json.dumps({
//!     "name": "blip", "duration": 0.3, "engine": 2,
//!     "root": {"type": "mul", "inputs": [
//!         {"type": "sine", "freq": 880},
//!         {"type": "env", "a": 0.002, "d": 0.08, "s": 0.0, "r": 0.05}]},
//! })
//! samples = tono.render(doc)        # list[float], mono
//! tono.play(doc, 0.4)               # hear it through the speakers
//! ```

use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;
use tono_core::dsl::SoundDoc;
use tono_core::render as engine;
use tono_core::streaming;

fn parse(doc_json: &str, sample_rate: Option<u32>) -> PyResult<SoundDoc> {
    let mut doc: SoundDoc = serde_json::from_str(doc_json)
        .map_err(|e| PyValueError::new_err(format!("invalid SoundDoc JSON: {e}")))?;
    if let Some(sr) = sample_rate {
        doc.sample_rate = sr;
    }
    Ok(doc)
}

/// Render a `SoundDoc` (JSON string) to mono `f32` samples. Deterministic:
/// `render(doc)` always returns the same samples.
#[pyfunction]
#[pyo3(signature = (doc_json, sample_rate=None))]
fn render(doc_json: &str, sample_rate: Option<u32>) -> PyResult<Vec<f32>> {
    Ok(engine::render(&parse(doc_json, sample_rate)?))
}

/// Render a `SoundDoc` to `(left, right)` stereo channels.
#[pyfunction]
#[pyo3(signature = (doc_json, sample_rate=None))]
fn render_stereo(doc_json: &str, sample_rate: Option<u32>) -> PyResult<(Vec<f32>, Vec<f32>)> {
    let product = engine::render_product(&parse(doc_json, sample_rate)?);
    Ok(match product.stereo {
        Some((l, r)) => (l, r),
        None => (product.mono.clone(), product.mono),
    })
}

/// The sample rate a doc renders at.
#[pyfunction]
fn sample_rate(doc_json: &str) -> PyResult<u32> {
    Ok(parse(doc_json, None)?.sample_rate)
}

/// Whether a doc can be streamed byte-identically in real time.
#[pyfunction]
fn is_streamable(doc_json: &str) -> PyResult<bool> {
    Ok(streaming::is_streamable(&parse(doc_json, None)?))
}

/// Play a `SoundDoc` through the default output device for `secs` seconds
/// (blocking, releasing the GIL). Errors if there is no audio device.
#[pyfunction]
#[pyo3(signature = (doc_json, secs))]
fn play(py: Python<'_>, doc_json: &str, secs: f32) -> PyResult<()> {
    let doc = parse(doc_json, None)?;
    py.allow_threads(|| tono_play::play_doc(&doc, secs))
        .map_err(|e| PyValueError::new_err(e.to_string()))
}

/// tono — deterministic sound synthesis for Python.
#[pymodule]
fn tono(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(render, m)?)?;
    m.add_function(wrap_pyfunction!(render_stereo, m)?)?;
    m.add_function(wrap_pyfunction!(sample_rate, m)?)?;
    m.add_function(wrap_pyfunction!(is_streamable, m)?)?;
    m.add_function(wrap_pyfunction!(play, m)?)?;
    Ok(())
}
