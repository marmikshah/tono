//! The performance runtime face (ADR 0005): `tono.Performance` runs a compiled
//! `Program` — a sample-accurate transport, a bounded scheduled-command queue,
//! stingers, crossfaded program swaps, metrics, and command capture — live on
//! the speakers, or headless for tests, servers, and CI.
//!
//! The live mode mirrors the [`stream`] architecture exactly: the core
//! `Performance` is the audio source behind an `spsc` split — the audio thread
//! drains the ring lock-free, a pump thread keeps it fed, and every Python
//! control method takes the *same* pump lock only briefly to schedule. The
//! headless mode keeps the identical control path but spawns no threads:
//! `fill` drives the render manually.
//!
//! This API is **stable** — frozen at 1.10.0-rc.1 (docs/api-tiers.md).

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread::{self, JoinHandle};

use numpy::{IntoPyArray, PyArray2, PyArrayMethods};
use pyo3::exceptions::{PyRuntimeError, PyTypeError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::{PyDict, PyList, PyTuple};

use tono_core::runtime::performance::PerformanceSnapshot;
use tono_core::runtime::{
    At, AudioSource, Command, Performance as CorePerformance,
    PerformanceError as CorePerformanceError, Pump, TransportState, spsc,
};

use crate::song::{Program, TonoError};
use crate::stream::{PUMP_TICK, RING_FRAMES, parse_doc, run_stream};

pyo3::create_exception!(
    tono,
    PerformanceError,
    TonoError,
    "A scheduling/runtime failure of a `Performance`. Base class of the \
     specific failures; a rejected swap target (`BadProgram`) raises this \
     directly."
);
pyo3::create_exception!(
    tono,
    QueueFullError,
    PerformanceError,
    "The scheduled-command queue is full — the command was rejected (and \
     counted in `metrics()['commands_dropped']`)."
);
pyo3::create_exception!(
    tono,
    UnknownPositionError,
    PerformanceError,
    "An `at` or `transition` named a marker or section the program doesn't \
     have."
);

/// The shared control surface: the running performance behind one lock,
/// produced into the ring. Held by the pump thread and every Python control
/// handle — the same shape as the live Engine's.
type Shared = Arc<Mutex<Pump<CorePerformance>>>;

/// Lock the shared pump, tolerating a poisoned mutex (a panicked holder leaves
/// the performance in a valid, if stale, state — never a reason to crash the
/// caller).
fn lock(shared: &Shared) -> MutexGuard<'_, Pump<CorePerformance>> {
    shared.lock().unwrap_or_else(|e| e.into_inner())
}

/// Map a core scheduling failure to the Python error surface: a full queue and
/// an unknown position get their own subclasses (the two failures a host can
/// react to programmatically); a rejected swap target is the base error.
fn perf_err(err: CorePerformanceError) -> PyErr {
    match err {
        CorePerformanceError::QueueFull => QueueFullError::new_err(err.to_string()),
        CorePerformanceError::UnknownPosition(_) => UnknownPositionError::new_err(err.to_string()),
        CorePerformanceError::BadProgram(_) => PerformanceError::new_err(err.to_string()),
    }
}

/// The transport state as its Python string.
fn state_str(state: TransportState) -> &'static str {
    match state {
        TransportState::Playing => "playing",
        TransportState::Paused => "paused",
        TransportState::Stopped => "stopped",
    }
}

/// A readable one-line rendering of a captured command — the capture dicts are
/// an inspection record, so `Swap`/`Stinger` payloads render as summaries,
/// not data.
fn command_label(command: &Command) -> String {
    match command {
        Command::Play => "Play".into(),
        Command::Pause => "Pause".into(),
        Command::Stop => "Stop".into(),
        Command::SeekBeat(beat) => format!("SeekBeat({beat})"),
        Command::SeekBar(bar) => format!("SeekBar({bar})"),
        Command::SeekSection(name) => format!("SeekSection({name:?})"),
        Command::SetLoopBars(start, end) => format!("SetLoopBars({start}, {end})"),
        Command::ClearLoop => "ClearLoop".into(),
        Command::SetGain(gain) => format!("SetGain({gain})"),
        Command::Swap(program) => format!("Swap({:?})", program.doc.name),
        Command::Stinger { gain, .. } => format!("Stinger(gain={gain})"),
    }
}

/// Where a `Performance` command lands on the timeline. Never constructed
/// directly — build one with the module helpers (`tono.next_bar()`,
/// `tono.next_beat()`, `tono.at_frame(n)`, `tono.at_beat(x)`, `tono.at_bar(n)`,
/// `tono.at_section(name)`, `tono.at_marker(name)`) and pass it as `at=`.
#[pyclass(module = "tono", name = "At", frozen)]
struct PyAt {
    inner: At,
}

#[pymethods]
impl PyAt {
    fn __repr__(&self) -> String {
        match &self.inner {
            At::Immediate => "at.immediate".to_string(), // None — never a PyAt
            At::Frame(n) => format!("at_frame({n})"),
            At::Beat(b) => format!("at_beat({b})"),
            At::Bar(n) => format!("at_bar({n})"),
            At::NextBeat => "next_beat()".to_string(),
            At::NextBar => "next_bar()".to_string(),
            At::Marker(name) => format!("at_marker({name:?})"),
            At::Section(name) => format!("at_section({name:?})"),
        }
    }
}

/// At the next bar line after the current position.
#[pyfunction]
fn next_bar() -> PyAt {
    PyAt { inner: At::NextBar }
}

/// At the next whole beat after the current position.
#[pyfunction]
fn next_beat() -> PyAt {
    PyAt {
        inner: At::NextBeat,
    }
}

/// At an absolute frame.
#[pyfunction]
fn at_frame(n: u64) -> PyAt {
    PyAt {
        inner: At::Frame(n),
    }
}

/// At an absolute beat (through the program's tempo map).
#[pyfunction]
fn at_beat(beat: f64) -> PyResult<PyAt> {
    if !beat.is_finite() {
        return Err(PyValueError::new_err(format!(
            "beat must be finite, got {beat}"
        )));
    }
    Ok(PyAt {
        inner: At::Beat(beat),
    })
}

/// At an absolute bar (through the program's meter map and pickup).
#[pyfunction]
fn at_bar(n: u32) -> PyAt {
    PyAt { inner: At::Bar(n) }
}

/// At a named section's first bar (an unknown name fails at schedule time
/// with `UnknownPositionError`).
#[pyfunction]
fn at_section(name: &str) -> PyAt {
    PyAt {
        inner: At::Section(name.to_owned()),
    }
}

/// At a named marker's position (an unknown name fails at schedule time with
/// `UnknownPositionError`).
#[pyfunction]
fn at_marker(name: &str) -> PyAt {
    PyAt {
        inner: At::Marker(name.to_owned()),
    }
}

/// Resolve the `at` argument shared by every control method: None means
/// immediate, a helper-built `At` carries its position, anything else is a
/// TypeError naming the accepted forms.
fn resolve_at(at: Option<&Bound<'_, PyAny>>) -> PyResult<At> {
    match at {
        None => Ok(At::Immediate),
        Some(obj) => match obj.cast::<PyAt>() {
            Ok(cell) => Ok(cell.borrow().inner.clone()),
            Err(_) => {
                let type_name = obj
                    .get_type()
                    .name()
                    .map(|n| n.to_string())
                    .unwrap_or_else(|_| "?".into());
                Err(PyTypeError::new_err(format!(
                    "at must be None (immediate) or built by tono.next_bar(), \
                     tono.next_beat(), tono.at_frame(n), tono.at_beat(x), \
                     tono.at_bar(n), tono.at_section(name), or tono.at_marker(name) \
                     — got {type_name:?}"
                )))
            }
        },
    }
}

/// A running `Program`: play, seek, loop, ride gain, fire stingers, and swap
/// programs — each scheduled at a frame, beat, bar, marker, or section, and
/// executed at the exact frame on the render side, so Python never wakes on a
/// musical boundary.
///
/// Live mode (the default) opens an output stream at the program's sample
/// rate; `headless=True` opens nothing and is driven manually with `fill` —
/// everything else works identically, so tests and CI need no audio device.
#[pyclass(module = "tono")]
struct Performance {
    shared: Shared,
    sample_rate: u32,
    stop: Arc<AtomicBool>,
    audio: Option<JoinHandle<()>>,
    pump: Option<JoinHandle<()>>,
    headless: bool,
}

impl Performance {
    /// The one scheduling path every control method takes: resolve `at`, a
    /// brief lock, schedule, map the error.
    fn schedule(&self, command: Command, at: Option<&Bound<'_, PyAny>>) -> PyResult<u64> {
        let at = resolve_at(at)?;
        lock(&self.shared).schedule(command, at).map_err(perf_err)
    }

    /// Stop the threads (idempotent — `close` and `Drop` run this same path,
    /// mirroring the live Engine's teardown).
    fn shutdown(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(h) = self.pump.take() {
            let _ = h.join();
        }
        if let Some(h) = self.audio.take() {
            let _ = h.join();
        }
    }
}

#[pymethods]
impl Performance {
    /// Run `program`. The program's compiled sample rate is authoritative —
    /// the core renders at it — so `sample_rate` may only restate it (any
    /// other value is a ValueError; recompile the program instead). With
    /// `headless=True` no device is opened and `fill` renders manually.
    #[new]
    #[pyo3(signature = (program, sample_rate=None, headless=false))]
    fn new(
        program: PyRef<'_, Program>,
        sample_rate: Option<u32>,
        headless: bool,
    ) -> PyResult<Self> {
        let program = program.shared();
        let program_rate = program.doc.sample_rate;
        match sample_rate {
            // The same range SoundDoc::validate enforces, like the Engine.
            Some(sr) if !(8_000..=192_000).contains(&sr) => {
                return Err(PyValueError::new_err(format!(
                    "sample_rate must be in [8000, 192000] Hz, got {sr}"
                )));
            }
            Some(sr) if sr != program_rate => {
                return Err(PyValueError::new_err(format!(
                    "the program's sample rate ({program_rate} Hz) is authoritative — a \
                     Performance renders the program at its own rate; recompile for {sr} Hz"
                )));
            }
            _ => {}
        }

        let perf = CorePerformance::new(program);
        let stop = Arc::new(AtomicBool::new(false));

        if headless {
            // No stream, no pump thread: the core Performance sits behind the
            // shared lock and `fill` drives it manually. The spsc split is
            // kept so the control path is identical to live mode's (the
            // renderer half is dropped; the ring is never used).
            let (pump, _renderer) = spsc(perf, RING_FRAMES);
            return Ok(Performance {
                shared: Arc::new(Mutex::new(pump)),
                sample_rate: program_rate,
                stop,
                audio: None,
                pump: None,
                headless: true,
            });
        }

        let (pump, renderer) = spsc(perf, RING_FRAMES);
        let shared: Shared = Arc::new(Mutex::new(pump));

        // Audio thread: owns the cpal stream, drains the ring lock-free.
        let (ready_tx, ready_rx) = mpsc::channel();
        let audio = {
            let stop = stop.clone();
            thread::Builder::new()
                .name("tono-audio".into())
                .spawn(move || run_stream(program_rate, renderer, stop, ready_tx))
                .map_err(|e| PyRuntimeError::new_err(e.to_string()))?
        };
        match ready_rx.recv() {
            Ok(Ok(())) => {}
            Ok(Err(e)) => {
                stop.store(true, Ordering::Relaxed);
                let _ = audio.join();
                return Err(PyRuntimeError::new_err(e));
            }
            Err(_) => return Err(PyRuntimeError::new_err("audio thread exited before start")),
        }

        // Pump thread: keeps the ring fed off the audio thread.
        let pump = {
            let shared = shared.clone();
            let stop_pump = stop.clone();
            let spawned = thread::Builder::new()
                .name("tono-pump".into())
                .spawn(move || {
                    while !stop_pump.load(Ordering::Relaxed) {
                        lock(&shared).pump(RING_FRAMES);
                        thread::sleep(PUMP_TICK);
                    }
                });
            match spawned {
                Ok(handle) => handle,
                Err(e) => {
                    // The audio thread is already live and playing the stream —
                    // tear it down so it isn't leaked on this error path.
                    stop.store(true, Ordering::Relaxed);
                    let _ = audio.join();
                    return Err(PyRuntimeError::new_err(e.to_string()));
                }
            }
        };

        Ok(Performance {
            shared,
            sample_rate: program_rate,
            stop,
            audio: Some(audio),
            pump: Some(pump),
            headless: false,
        })
    }

    /// The program's sample rate (Hz) — the rate the performance renders at.
    #[getter]
    fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    /// The transport state: "playing", "paused", or "stopped". Reads the
    /// render-side transport, so a just-scheduled command shows after the
    /// next rendered frame (in live mode: within a few ms).
    #[getter]
    fn state(&self) -> &'static str {
        state_str(lock(&self.shared).transport().state())
    }

    /// The transport position in frames.
    #[getter]
    fn position_frames(&self) -> u64 {
        lock(&self.shared).transport().position_frames()
    }

    /// The transport position in beats (through the tempo map).
    #[getter]
    fn position_beats(&self) -> f64 {
        lock(&self.shared).transport().position_beats()
    }

    /// The transport position in bars (through the meter map and pickup).
    #[getter]
    fn position_bars(&self) -> f64 {
        lock(&self.shared).transport().position_bars()
    }

    /// Commands queued but not yet executed.
    #[getter]
    fn queue_depth(&self) -> usize {
        lock(&self.shared).queue_depth()
    }

    /// Start or resume playback. Returns the scheduled command's sequence id.
    #[pyo3(signature = (at=None))]
    fn play(&self, at: Option<&Bound<'_, PyAny>>) -> PyResult<u64> {
        self.schedule(Command::Play, at)
    }

    /// Hold the transport (position kept). Returns the sequence id.
    #[pyo3(signature = (at=None))]
    fn pause(&self, at: Option<&Bound<'_, PyAny>>) -> PyResult<u64> {
        self.schedule(Command::Pause, at)
    }

    /// Stop and rewind to the start. Returns the sequence id.
    #[pyo3(signature = (at=None))]
    fn stop(&self, at: Option<&Bound<'_, PyAny>>) -> PyResult<u64> {
        self.schedule(Command::Stop, at)
    }

    /// Seek to `beat` (the song source follows, deterministically). Returns
    /// the sequence id.
    #[pyo3(signature = (beat, at=None))]
    fn seek_beat(&self, beat: f64, at: Option<&Bound<'_, PyAny>>) -> PyResult<u64> {
        if !beat.is_finite() {
            return Err(PyValueError::new_err(format!(
                "beat must be finite, got {beat}"
            )));
        }
        self.schedule(Command::SeekBeat(beat), at)
    }

    /// Seek to `bar`. Returns the sequence id.
    #[pyo3(signature = (bar, at=None))]
    fn seek_bar(&self, bar: u32, at: Option<&Bound<'_, PyAny>>) -> PyResult<u64> {
        self.schedule(Command::SeekBar(bar), at)
    }

    /// Loop the bar range `[start, end)`. Returns the sequence id.
    #[pyo3(signature = (start, end, at=None))]
    fn set_loop_bars(&self, start: u32, end: u32, at: Option<&Bound<'_, PyAny>>) -> PyResult<u64> {
        self.schedule(Command::SetLoopBars(start, end), at)
    }

    /// Clear the loop. Returns the sequence id.
    #[pyo3(signature = (at=None))]
    fn clear_loop(&self, at: Option<&Bound<'_, PyAny>>) -> PyResult<u64> {
        self.schedule(Command::ClearLoop, at)
    }

    /// Set the master gain (clamped to 0..2 by the core, ramped click-free).
    /// Returns the sequence id.
    #[pyo3(signature = (gain, at=None))]
    fn set_gain(&self, gain: f32, at: Option<&Bound<'_, PyAny>>) -> PyResult<u64> {
        if !gain.is_finite() {
            return Err(PyValueError::new_err(format!(
                "gain must be finite, got {gain}"
            )));
        }
        self.schedule(Command::SetGain(gain), at)
    }

    /// A quantized section transition: seek to the section's first bar at `at`
    /// (default `tono.next_bar()`-style scheduling is the caller's choice —
    /// pass `at=tono.next_bar()`). The latest transition wins while one is
    /// pending; an unknown section raises `UnknownPositionError` now, never a
    /// silent no-op later. Returns the sequence id.
    #[pyo3(signature = (name, at=None))]
    fn transition(&self, name: &str, at: Option<&Bound<'_, PyAny>>) -> PyResult<u64> {
        let at = resolve_at(at)?;
        lock(&self.shared)
            .transition_to_section(name, at)
            .map_err(perf_err)
    }

    /// Fire a one-shot SoundDoc (JSON) over the song at `at`, at `gain`. The
    /// doc is parsed, validated, and rendered at schedule time — never on the
    /// audio path. This JSON bridge is the documented way to fire one-shots
    /// until the typed API grows SoundDoc authoring. Returns the sequence id.
    #[pyo3(signature = (doc_json, gain=1.0, at=None))]
    fn stinger(&self, doc_json: &str, gain: f32, at: Option<&Bound<'_, PyAny>>) -> PyResult<u64> {
        if !gain.is_finite() {
            return Err(PyValueError::new_err(format!(
                "gain must be finite, got {gain}"
            )));
        }
        let doc = parse_doc(doc_json, self.sample_rate)?;
        let at = resolve_at(at)?;
        lock(&self.shared).stinger(&doc, gain, at).map_err(perf_err)
    }

    /// Swap to another program at `at` — the new program crossfades in from
    /// its frame 0 with its own transport. The target must share the running
    /// program's sample rate (a mismatched target is a ValueError — recompile
    /// it); a target the core rejects raises `PerformanceError` and the last
    /// valid program keeps running. Returns the sequence id.
    #[pyo3(signature = (program, at=None))]
    fn swap(&self, program: PyRef<'_, Program>, at: Option<&Bound<'_, PyAny>>) -> PyResult<u64> {
        let program = program.shared();
        if program.doc.sample_rate != self.sample_rate {
            return Err(PyValueError::new_err(format!(
                "swap target runs at {} Hz but the performance renders at {} Hz — \
                 recompile the target at the performance's rate",
                program.doc.sample_rate, self.sample_rate
            )));
        }
        let at = resolve_at(at)?;
        lock(&self.shared).swap_to(program, at).map_err(perf_err)
    }

    /// A health snapshot: frames_rendered, commands_executed,
    /// commands_dropped, queue_depth_max, swaps, stingers_fired, and the
    /// queue_depth sampled now.
    fn metrics<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyDict>> {
        let guard = lock(&self.shared);
        let m = guard.metrics();
        let dict = PyDict::new(py);
        dict.set_item("frames_rendered", m.frames_rendered)?;
        dict.set_item("commands_executed", m.commands_executed)?;
        dict.set_item("commands_dropped", m.commands_dropped)?;
        dict.set_item("queue_depth_max", m.queue_depth_max)?;
        dict.set_item("swaps", m.swaps)?;
        dict.set_item("stingers_fired", m.stingers_fired)?;
        dict.set_item("queue_depth", guard.queue_depth())?;
        Ok(dict)
    }

    /// Start recording scheduled commands for deterministic replay.
    fn capture_start(&self) {
        lock(&self.shared).start_capture();
    }

    /// Stop recording and return the captured commands — one dict per command,
    /// `{at_frame, seq, command}` with `command` rendered readably (e.g.
    /// `"Play"`, `"SetGain(0.5)"`, `"SeekBar(1)"`). The record is for
    /// inspection; replaying is re-issuing the same calls (the render is
    /// deterministic, so the take reproduces bit-exactly).
    fn capture_stop<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyList>> {
        let captured = lock(&self.shared).stop_capture();
        let list = PyList::empty(py);
        for c in captured {
            let dict = PyDict::new(py);
            dict.set_item("at_frame", c.at_frame)?;
            dict.set_item("seq", c.seq)?;
            dict.set_item("command", command_label(&c.command))?;
            list.append(dict)?;
        }
        Ok(list)
    }

    /// A control-state snapshot dict: `position_frames`, `state`,
    /// `master_gain`, `loop_range` (None or a `(start, end)` frame pair).
    /// `apply_snapshot` returns the performance to exactly this state.
    fn snapshot<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyDict>> {
        let snap = lock(&self.shared).snapshot();
        let dict = PyDict::new(py);
        dict.set_item("position_frames", snap.position)?;
        dict.set_item("state", state_str(snap.state))?;
        dict.set_item("master_gain", snap.master_gain)?;
        match snap.loop_range {
            Some((start, end)) => dict.set_item("loop_range", (start, end))?,
            None => dict.set_item("loop_range", py.None())?,
        }
        Ok(dict)
    }

    /// Restore a `snapshot()` dict (the song source re-seeks, deterministically).
    /// Missing or mistyped keys are ValueErrors.
    fn apply_snapshot(&self, snapshot: &Bound<'_, PyDict>) -> PyResult<()> {
        let get = |key: &str| -> PyResult<Bound<'_, PyAny>> {
            snapshot
                .get_item(key)?
                .ok_or_else(|| PyValueError::new_err(format!("snapshot is missing '{key}'")))
        };
        let position = get("position_frames")?
            .extract::<u64>()
            .map_err(|_| PyValueError::new_err("snapshot['position_frames'] must be an int"))?;
        let state_obj = get("state")?;
        let state = match state_obj
            .extract::<String>()
            .map_err(|_| PyValueError::new_err("snapshot['state'] must be a string"))?
            .as_str()
        {
            "playing" => TransportState::Playing,
            "paused" => TransportState::Paused,
            "stopped" => TransportState::Stopped,
            other => {
                return Err(PyValueError::new_err(format!(
                    "snapshot['state'] must be \"playing\", \"paused\", or \"stopped\", \
                     got {other:?}"
                )));
            }
        };
        let master_gain = get("master_gain")?
            .extract::<f32>()
            .map_err(|_| PyValueError::new_err("snapshot['master_gain'] must be a number"))?;
        let loop_obj = get("loop_range")?;
        let loop_range = if loop_obj.is_none() {
            None
        } else {
            Some(loop_obj.extract::<(u64, u64)>().map_err(|_| {
                PyValueError::new_err("snapshot['loop_range'] must be None or a (start, end) pair")
            })?)
        };
        lock(&self.shared).apply_snapshot(&PerformanceSnapshot {
            position,
            state,
            master_gain,
            loop_range,
        });
        Ok(())
    }

    /// Headless mode only: render `frames` frames into an owned stereo
    /// `(frames, 2)` float32 array, executing due commands at their exact
    /// frames. The GIL is released for the render. On a live Performance this
    /// raises RuntimeError — the output device is the sink there.
    fn fill<'py>(&self, py: Python<'py>, frames: usize) -> PyResult<Bound<'py, PyArray2<f32>>> {
        if !self.headless {
            return Err(PyRuntimeError::new_err(
                "fill() is headless-only — a live Performance renders to the output device",
            ));
        }
        let interleaved = py.detach(|| {
            let mut buf = vec![0.0; frames * 2];
            lock(&self.shared).fill(&mut buf);
            buf
        });
        interleaved.into_pyarray(py).reshape([frames, 2])
    }

    /// Stop the stream and join the threads (idempotent; also run by Drop).
    /// Headless performances have no threads — `close` is a no-op there.
    fn close(&mut self) {
        self.shutdown();
    }

    fn __enter__(slf: PyRef<'_, Self>) -> PyRef<'_, Self> {
        slf
    }

    #[pyo3(signature = (*_args))]
    fn __exit__(&mut self, _args: &Bound<'_, PyTuple>) -> bool {
        self.shutdown();
        false
    }
}

impl Drop for Performance {
    fn drop(&mut self) {
        self.shutdown();
    }
}

/// Register the performance-runtime class, its `At` helpers, and its errors on
/// the module.
pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add("PerformanceError", m.py().get_type::<PerformanceError>())?;
    m.add("QueueFullError", m.py().get_type::<QueueFullError>())?;
    m.add(
        "UnknownPositionError",
        m.py().get_type::<UnknownPositionError>(),
    )?;
    m.add_class::<PyAt>()?;
    m.add_class::<Performance>()?;
    m.add_function(wrap_pyfunction!(next_bar, m)?)?;
    m.add_function(wrap_pyfunction!(next_beat, m)?)?;
    m.add_function(wrap_pyfunction!(at_frame, m)?)?;
    m.add_function(wrap_pyfunction!(at_beat, m)?)?;
    m.add_function(wrap_pyfunction!(at_bar, m)?)?;
    m.add_function(wrap_pyfunction!(at_section, m)?)?;
    m.add_function(wrap_pyfunction!(at_marker, m)?)?;
    Ok(())
}
