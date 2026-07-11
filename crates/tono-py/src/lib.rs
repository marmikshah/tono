//! tono — Python bindings for the deterministic tono audio engine.
//!
//! Two shapes over one engine, mirroring the Rust runtime:
//!
//! * **numpy pull** — [`render`] a `SoundDoc` JSON or [`Patch::render`] a
//!   parameterized patch straight to an `np.ndarray`. Deterministic, so it is
//!   testable in CI and drops into any audio callback, a WAV bounce, or a
//!   Jupyter cell.
//! * **owned stream** — a live [`Engine`] that owns a cpal output stream and a
//!   real-time render thread; Python pushes control (`note_on`, `set_param`,
//!   `set_intensity`, `trigger`) while the audio thread stays GIL-free.
//!
//! `SoundDoc`s and `Patch`es cross the boundary as JSON strings — the same serde
//! types the CLI and desktop studio use.

use std::collections::BTreeMap;

use numpy::{IntoPyArray, PyArray1};
use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;

use tono_core::dsl::SoundDoc;
use tono_core::patch::Patch as CorePatch;

mod stream;

/// Parse a `SoundDoc` from JSON, mapping serde/validation failures to a Python
/// `ValueError`.
fn parse_doc(json: &str) -> PyResult<SoundDoc> {
    let doc: SoundDoc =
        serde_json::from_str(json).map_err(|e| PyValueError::new_err(e.to_string()))?;
    doc.validate().map_err(PyValueError::new_err)?;
    // validate() is filesystem-free (the core is pure); the loader owns the
    // existence check so a missing SoundFont still fails loud at load time.
    for sf2 in doc.sf2_paths() {
        if !std::path::Path::new(sf2).exists() {
            return Err(PyValueError::new_err(format!(
                "seq.sf2: no such file '{sf2}'"
            )));
        }
    }
    Ok(doc)
}

/// Render a `SoundDoc` (as a JSON string) to a mono `float32` numpy array.
///
/// A pure function of the graph, seed, and sample rate — the same bytes every
/// call, on a given platform. Feed the result to `sounddevice`, a Pygame
/// buffer, a WAV writer, or an assertion.
#[pyfunction]
fn render<'py>(py: Python<'py>, doc_json: &str) -> PyResult<Bound<'py, PyArray1<f32>>> {
    let doc = parse_doc(doc_json)?;
    let signal = py.detach(|| tono_core::render::render(&doc));
    Ok(signal.into_pyarray(py))
}

/// A zero-asset SFX patch: a graph plus named parameters. Load it once, then
/// render infinite per-instance variations by naming parameter values.
///
/// ```python
/// import tono, numpy as np
/// p = tono.Patch(open("impact.patch.json").read())
/// buf = p.render(hardness=0.7, size=0.3)   # -> np.float32 ndarray
/// ```
#[pyclass]
struct Patch {
    inner: CorePatch,
}

#[pymethods]
impl Patch {
    /// Load a patch from its JSON definition.
    #[new]
    fn new(json: &str) -> PyResult<Self> {
        let inner: CorePatch =
            serde_json::from_str(json).map_err(|e| PyValueError::new_err(e.to_string()))?;
        Ok(Patch { inner })
    }

    /// Render the patch to a mono `float32` numpy array. Named keyword arguments
    /// set parameters; any omitted parameter falls back to its default.
    #[pyo3(signature = (**params))]
    fn render<'py>(
        &self,
        py: Python<'py>,
        params: Option<BTreeMap<String, f32>>,
    ) -> PyResult<Bound<'py, PyArray1<f32>>> {
        let mut values = self.inner.defaults();
        if let Some(overrides) = params {
            values.extend(overrides);
        }
        let signal = py
            .detach(|| self.inner.render(&values))
            .map_err(PyValueError::new_err)?;
        Ok(signal.into_pyarray(py))
    }

    /// The patch's parameter names mapped to their default values.
    fn defaults(&self) -> BTreeMap<String, f32> {
        self.inner.defaults()
    }
}

/// The `tono` Python module.
#[pymodule]
fn tono(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(render, m)?)?;
    m.add_class::<Patch>()?;
    stream::register(m)?;
    Ok(())
}
