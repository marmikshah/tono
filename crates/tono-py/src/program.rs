//! `tono.Program`: the compiled, hashed song artifact and its renders.

use std::sync::Arc;

use numpy::{IntoPyArray, PyArray1, PyArray2, PyArrayMethods};
use pyo3::exceptions::{PyOSError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::{PyDict, PyList};

use tono_core::program::Program as CoreProgram;

use crate::convert::{TonoError, diagnostics_list, wave_str};

/// A compiled song: validated, resolved, hashed — the immutable artifact
/// applications render and ship.
#[pyclass(module = "tono")]
pub(crate) struct Program {
    /// Shared: a `Performance` Arc-clones the program as its running (and
    /// swap-target) artifact.
    pub(crate) inner: Arc<CoreProgram>,
}

impl Program {
    /// The shared inner program, for `Performance::new` / `swap_to`.
    pub(crate) fn shared(&self) -> Arc<CoreProgram> {
        self.inner.clone()
    }
}

#[pymethods]
impl Program {
    /// The canonical content hash: equivalent songs hash equal, from Rust or
    /// Python alike.
    #[getter]
    fn hash(&self) -> u64 {
        self.inner.hash
    }

    /// The sample rate the program was compiled for.
    #[getter]
    fn sample_rate(&self) -> u32 {
        self.inner.doc.sample_rate
    }

    /// The total duration in seconds, including the release/reverb tail.
    #[getter]
    fn duration_seconds(&self) -> f32 {
        self.inner.doc.duration
    }

    /// One dict per track, in declaration order: index, name, wave, notes, mute,
    /// solo.
    #[getter]
    fn tracks<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyList>> {
        let list = PyList::empty(py);
        for (index, t) in self.inner.meta.tracks.iter().enumerate() {
            let dict = PyDict::new(py);
            dict.set_item("index", index)?;
            dict.set_item("name", &t.name)?;
            dict.set_item("wave", wave_str(t.wave))?;
            dict.set_item("notes", t.notes)?;
            dict.set_item("mute", t.mute)?;
            dict.set_item("solo", t.solo)?;
            list.append(dict)?;
        }
        Ok(list)
    }

    /// Bounded estimates of what the program costs to render or run:
    /// frames, events, peak_voices, memory_bytes.
    #[getter]
    fn estimates<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyDict>> {
        let dict = PyDict::new(py);
        dict.set_item("frames", self.inner.estimates.frames)?;
        dict.set_item("events", self.inner.estimates.events)?;
        dict.set_item("peak_voices", self.inner.estimates.peak_voices)?;
        dict.set_item("memory_bytes", self.inner.estimates.memory_bytes)?;
        Ok(dict)
    }

    /// Offline compile warnings (the streaming blockers), as diagnostic dicts
    /// of the same shape as `CompileError.diagnostics`.
    #[getter]
    fn warnings<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyList>> {
        diagnostics_list(py, &self.inner.warnings())
    }

    /// Whether the resolved document streams natively.
    #[getter]
    fn is_streamable(&self) -> bool {
        self.inner.is_streamable()
    }

    /// Render the full program to a stereo `np.ndarray` of shape
    /// `(frames, 2)`, dtype `float32`, C-order, channel order L/R. The array
    /// is an owned copy: safe to keep or mutate, and rendering again yields
    /// the same bytes.
    fn render<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyArray2<f32>>> {
        let (left, right) = py.detach(|| self.inner.render_stereo());
        let frames = left.len();
        let mut interleaved = Vec::with_capacity(frames * 2);
        for i in 0..frames {
            interleaved.push(left[i]);
            interleaved.push(right[i]);
        }
        interleaved.into_pyarray(py).reshape([frames, 2])
    }

    /// Render the full program to a mono `np.ndarray` of shape `(frames,)`,
    /// dtype `float32` — the mid of the stereo render, an owned copy.
    fn render_mono<'py>(&self, py: Python<'py>) -> Bound<'py, PyArray1<f32>> {
        py.detach(|| self.inner.render_mono()).into_pyarray(py)
    }

    /// Render a selected range as a stereo `(frames, 2)` float32 array — a
    /// slice of the full render, so tails crossing the boundary sound
    /// exactly as in the full mix. Pass `frames=(start, end)` or
    /// `bars=(start_bar, end_bar)` (through the program's meter map).
    #[pyo3(signature = (*, frames=None, bars=None))]
    fn render_range<'py>(
        &self,
        py: Python<'py>,
        frames: Option<(u64, u64)>,
        bars: Option<(u32, u32)>,
    ) -> PyResult<Bound<'py, PyArray2<f32>>> {
        match (frames, bars) {
            (Some((s, e)), None) => {
                let (l, r) = py.detach(|| self.inner.render_range_frames(s, e));
                let n = l.len();
                let mut interleaved = Vec::with_capacity(n * 2);
                for i in 0..n {
                    interleaved.push(l[i]);
                    interleaved.push(r[i]);
                }
                interleaved.into_pyarray(py).reshape([n, 2])
            }
            (None, Some((s, e))) => {
                let (l, r) = py.detach(|| self.inner.render_range_bars(s, e));
                let n = l.len();
                let mut interleaved = Vec::with_capacity(n * 2);
                for i in 0..n {
                    interleaved.push(l[i]);
                    interleaved.push(r[i]);
                }
                interleaved.into_pyarray(py).reshape([n, 2])
            }
            _ => Err(PyValueError::new_err(
                "pass exactly one of frames=(start, end) or bars=(start_bar, end_bar)",
            )),
        }
    }

    /// Render per-track and per-bus stereo stems (pre-master-chain): a dict
    /// mapping stem id (`"bass"`, `"bus:verb"`) to an `(frames, 2)` float32
    /// array, in declaration order. A stem's `bus` routing is in
    /// `stem_routing`. Muted tracks are silent stems.
    fn render_stems<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyDict>> {
        let stems = py.detach(|| self.inner.render_stems());
        let dict = PyDict::new(py);
        for s in &stems {
            let frames = s.left.len();
            let mut interleaved = Vec::with_capacity(frames * 2);
            for i in 0..frames {
                interleaved.push(s.left[i]);
                interleaved.push(s.right[i]);
            }
            dict.set_item(&s.id, interleaved.into_pyarray(py).reshape([frames, 2])?)?;
        }
        Ok(dict)
    }

    /// Where each track stem routes: `{track_id: bus_id}` for tracks whose
    /// main output goes to a bus (their stem is already inside that bus's
    /// stem). Read from the compiled document — no render pass.
    #[getter]
    fn stem_routing<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyDict>> {
        let dict = PyDict::new(py);
        if let tono_core::dsl::Node::Tracks { tracks, .. } = &self.inner.doc.root {
            for t in tracks {
                if let (Some(id), Some(bus)) = (&t.id, &t.bus) {
                    dict.set_item(id, bus)?;
                }
            }
        }
        Ok(dict)
    }

    /// Serialize the program bundle (compact JSON).
    fn to_json(&self) -> String {
        self.inner.to_json()
    }

    /// Load a program bundle: rejects bundles newer than this binary (T3001)
    /// and re-verifies the content hash (T3002).
    #[staticmethod]
    fn from_json(json: &str) -> PyResult<Self> {
        CoreProgram::from_json(json)
            .map(|inner| Program {
                inner: Arc::new(inner),
            })
            .map_err(|e| TonoError::new_err(e.to_string()))
    }

    /// Write the program bundle to `path` (see `to_json`).
    fn save(&self, path: &str) -> PyResult<()> {
        std::fs::write(path, self.inner.to_json()).map_err(|e| PyOSError::new_err(e.to_string()))
    }

    /// Read a program bundle from `path` (see `from_json`).
    #[staticmethod]
    fn load(path: &str) -> PyResult<Self> {
        let json = std::fs::read_to_string(path).map_err(|e| PyOSError::new_err(e.to_string()))?;
        Program::from_json(&json)
    }

    fn __repr__(&self) -> String {
        format!(
            "Program('{}', hash={:#018x}, tracks={})",
            self.inner.doc.name,
            self.inner.hash,
            self.inner.meta.tracks.len()
        )
    }
}

/// Register `Program`.
pub(crate) fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<Program>()
}
