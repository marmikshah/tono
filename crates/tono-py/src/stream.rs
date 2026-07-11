//! The owned-stream live API: a real-time [`Engine`] that owns a cpal output
//! stream and a render thread, driven live from Python.
//!
//! Threading (the seam that keeps the audio thread GIL-free):
//!
//! * The **audio thread** owns the cpal stream; its callback holds a [`Renderer`]
//!   and drains the wait-free sample ring — no lock, no Python, no allocation.
//! * A **pump thread** (pure Rust) locks the shared [`Pump`] and renders blocks
//!   into the ring, keeping the audio thread fed.
//! * **Python control** (`note_on`, `set_param`, `set_intensity`, `trigger`)
//!   locks the *same* `Pump` only briefly to mutate a source via
//!   `Mixer::get_mut`. It shares that lock with the pump thread — never with the
//!   audio thread.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use cpal::SampleFormat;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use pyo3::exceptions::{PyRuntimeError, PyValueError};
use pyo3::prelude::*;

use tono_core::adaptive::{AdaptiveMusic as CoreAdaptive, LoopBuffer};
use tono_core::drumkit::DrumKit as CoreDrumKit;
use tono_core::dsl::SoundDoc;
use tono_core::instrument::{Instrument as CoreInstrument, Note};
use tono_core::patch::Patch as CorePatch;
use tono_core::presets;
use tono_core::runtime::{
    AudioSource, Engine as CoreEngine, Mixer, PatchId, Pump, Renderer, SourceId, Tween, spsc,
};

/// Ring depth (frames). ~85 ms at 48 kHz — ample underrun headroom.
const RING_FRAMES: usize = 4096;
/// How often the pump thread refills the ring.
const PUMP_TICK: Duration = Duration::from_millis(5);

/// The shared control surface: the whole mix behind one lock, produced into the
/// ring. Held by the pump thread and every Python control handle.
type Shared = Arc<Mutex<Pump<Mixer>>>;

/// Lock the shared pump, tolerating a poisoned mutex (a panicked holder leaves
/// the mix in a valid, if stale, state — never a reason to crash the caller).
fn lock(shared: &Shared) -> MutexGuard<'_, Pump<Mixer>> {
    shared.lock().unwrap_or_else(|e| e.into_inner())
}

/// Coerce a Python note argument — an `int` (MIDI 0..=127) or a name string
/// (`"C4"`, `"F#3"`, `"midi:60"`) — into a [`Note`].
fn to_note(note: &Bound<'_, PyAny>) -> PyResult<Note> {
    if let Ok(midi) = note.extract::<i64>() {
        if (0..=127).contains(&midi) {
            return Ok(Note::from(midi as u8));
        }
        return Err(PyValueError::new_err("MIDI note out of range 0..=127"));
    }
    if let Ok(name) = note.extract::<String>() {
        return Note::parse(&name)
            .ok_or_else(|| PyValueError::new_err(format!("unrecognized note name: {name:?}")));
    }
    Err(PyValueError::new_err(
        "note must be an int (MIDI) or a string like \"C4\"",
    ))
}

/// Parse a `SoundDoc` from JSON and stamp it with the engine's sample rate, so a
/// looped stem or stinger plays at the right pitch regardless of the rate baked
/// into its JSON.
fn parse_doc(json: &str, sample_rate: u32) -> PyResult<SoundDoc> {
    let mut doc: SoundDoc =
        serde_json::from_str(json).map_err(|e| PyValueError::new_err(e.to_string()))?;
    doc.sample_rate = sample_rate;
    doc.validate().map_err(PyValueError::new_err)?;
    Ok(doc)
}

/// The native output device's default sample rate.
fn device_sample_rate() -> PyResult<u32> {
    let host = cpal::default_host();
    let device = host
        .default_output_device()
        .ok_or_else(|| PyRuntimeError::new_err("no default audio output device"))?;
    let config = device
        .default_output_config()
        .map_err(|e| PyRuntimeError::new_err(e.to_string()))?;
    Ok(config.sample_rate().0)
}

/// Build and run the cpal output stream on the calling (dedicated) thread,
/// draining `renderer` in the callback. Reports build success/failure back over
/// `ready`, then keeps the stream alive until `stop` is set.
fn run_stream(
    sample_rate: u32,
    mut renderer: Renderer,
    stop: Arc<AtomicBool>,
    ready: mpsc::Sender<Result<(), String>>,
) {
    let built = (|| -> Result<cpal::Stream, String> {
        let host = cpal::default_host();
        let device = host
            .default_output_device()
            .ok_or("no default audio output device")?;
        let config = device.default_output_config().map_err(|e| e.to_string())?;
        if config.sample_format() != SampleFormat::F32 {
            return Err("only F32 output is supported".into());
        }
        let channels = config.channels() as usize;
        let mut stream_config: cpal::StreamConfig = config.into();
        stream_config.sample_rate = cpal::SampleRate(sample_rate);

        let mut scratch: Vec<f32> = Vec::new();
        let stream = device
            .build_output_stream(
                &stream_config,
                move |data: &mut [f32], _| {
                    // Never unwind into cpal's C callback (UB): contain any panic
                    // in the render path and fall back to silence.
                    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        let frames = data.len() / channels;
                        if scratch.len() < frames * 2 {
                            scratch.resize(frames * 2, 0.0);
                        }
                        let interleaved = &mut scratch[..frames * 2];
                        renderer.fill(interleaved);
                        tono_core::runtime::write_interleaved(data, channels, interleaved);
                    }));
                    if result.is_err() {
                        data.fill(0.0);
                    }
                },
                |err| eprintln!("tono audio stream error: {err}"),
                None,
            )
            .map_err(|e| e.to_string())?;
        stream.play().map_err(|e| e.to_string())?;
        Ok(stream)
    })();

    match built {
        Ok(stream) => {
            let _ = ready.send(Ok(()));
            // Keep the (!Send) stream alive on this thread until asked to stop.
            while !stop.load(Ordering::Relaxed) {
                thread::sleep(Duration::from_millis(50));
            }
            drop(stream);
        }
        Err(e) => {
            let _ = ready.send(Err(e));
        }
    }
}

/// A live audio engine that owns an output stream. Load instruments, a drum kit,
/// SFX patches, and an adaptive-music bed, then drive them from the game loop.
#[pyclass]
struct Engine {
    shared: Shared,
    /// The shared SFX sub-engine inside the mix — patch triggers play here.
    sfx: SourceId,
    sample_rate: u32,
    stop: Arc<AtomicBool>,
    audio: Option<JoinHandle<()>>,
    pump: Option<JoinHandle<()>>,
}

#[pymethods]
impl Engine {
    /// Open an engine on the default output device. `sample_rate` defaults to the
    /// device's native rate.
    #[new]
    #[pyo3(signature = (sample_rate=None))]
    fn new(sample_rate: Option<u32>) -> PyResult<Self> {
        let sample_rate = match sample_rate {
            Some(sr) => sr,
            None => device_sample_rate()?,
        };

        let mut mixer = Mixer::new();
        let sfx = mixer.add(CoreEngine::new(sample_rate));
        let (pump, renderer) = spsc(mixer, RING_FRAMES);
        let shared: Shared = Arc::new(Mutex::new(pump));
        let stop = Arc::new(AtomicBool::new(false));

        // Audio thread: owns the cpal stream, drains the ring lock-free.
        let (ready_tx, ready_rx) = mpsc::channel();
        let audio = {
            let stop = stop.clone();
            thread::Builder::new()
                .name("tono-audio".into())
                .spawn(move || run_stream(sample_rate, renderer, stop, ready_tx))
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
                    // tear it down so it isn't leaked (parked forever with the
                    // stream sounding) on this error path.
                    stop.store(true, Ordering::Relaxed);
                    let _ = audio.join();
                    return Err(PyRuntimeError::new_err(e.to_string()));
                }
            }
        };

        Ok(Engine {
            shared,
            sfx,
            sample_rate,
            stop,
            audio: Some(audio),
            pump: Some(pump),
        })
    }

    /// The engine's sample rate (Hz).
    #[getter]
    fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    /// Add a factory-preset instrument to the mix (e.g. `"warm_lead"`,
    /// `"sub_bass"`, `"fm_tine"`). See [`presets`] for the full list.
    fn instrument(&self, name: &str) -> PyResult<Instrument> {
        let design = presets::preset(name)
            .ok_or_else(|| PyValueError::new_err(format!("unknown instrument preset: {name}")))?;
        let inst = CoreInstrument::new(design, self.sample_rate)
            .map_err(|e| PyValueError::new_err(format!("{e:?}")))?;
        let id = lock(&self.shared).add(inst);
        Ok(Instrument {
            shared: self.shared.clone(),
            id,
        })
    }

    /// Add a General MIDI drum kit to the mix.
    fn drumkit(&self) -> DrumKit {
        let kit = CoreDrumKit::general_midi(self.sample_rate);
        let id = lock(&self.shared).add(kit);
        DrumKit {
            shared: self.shared.clone(),
            id,
        }
    }

    /// Add an adaptive-music bed to the mix.
    fn adaptive(&self) -> AdaptiveMusic {
        let music = CoreAdaptive::new(self.sample_rate);
        let id = lock(&self.shared).add(music);
        AdaptiveMusic {
            shared: self.shared.clone(),
            id,
            sample_rate: self.sample_rate,
        }
    }

    /// Load a zero-asset SFX patch (JSON). The returned handle triggers one-shot
    /// instances on the shared SFX engine, with per-trigger named parameters.
    fn load_patch(&self, json: &str) -> PyResult<PatchVoice> {
        let patch: CorePatch =
            serde_json::from_str(json).map_err(|e| PyValueError::new_err(e.to_string()))?;
        let mut guard = lock(&self.shared);
        let engine = guard
            .get_mut::<CoreEngine>(self.sfx)
            .ok_or_else(|| PyRuntimeError::new_err("SFX engine missing from the mix"))?;
        let id = engine.load_patch(&patch);
        Ok(PatchVoice {
            shared: self.shared.clone(),
            sfx: self.sfx,
            patch: id,
        })
    }
}

impl Drop for Engine {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(h) = self.pump.take() {
            let _ = h.join();
        }
        if let Some(h) = self.audio.take() {
            let _ = h.join();
        }
    }
}

/// A playable instrument voice in the mix.
#[pyclass]
struct Instrument {
    shared: Shared,
    id: SourceId,
}

#[pymethods]
impl Instrument {
    /// Strike a note (MIDI int or name). `velocity` is 0..1.
    #[pyo3(signature = (note, velocity=1.0))]
    fn note_on(&self, note: &Bound<'_, PyAny>, velocity: f32) -> PyResult<()> {
        let note = to_note(note)?;
        if let Some(inst) = lock(&self.shared).get_mut::<CoreInstrument>(self.id) {
            inst.note_on(note, velocity);
        }
        Ok(())
    }

    /// Release a held note.
    fn note_off(&self, note: &Bound<'_, PyAny>) -> PyResult<()> {
        let note = to_note(note)?;
        if let Some(inst) = lock(&self.shared).get_mut::<CoreInstrument>(self.id) {
            inst.note_off(note);
        }
        Ok(())
    }

    /// Set a named patch parameter live. Returns whether the parameter exists.
    fn set_param(&self, name: &str, value: f32) -> bool {
        lock(&self.shared)
            .get_mut::<CoreInstrument>(self.id)
            .map(|inst| inst.set_param(name, value))
            .unwrap_or(false)
    }

    /// Release every sounding voice.
    fn all_notes_off(&self) {
        if let Some(inst) = lock(&self.shared).get_mut::<CoreInstrument>(self.id) {
            inst.all_notes_off();
        }
    }
}

/// A General MIDI drum kit in the mix.
#[pyclass]
struct DrumKit {
    shared: Shared,
    id: SourceId,
}

#[pymethods]
impl DrumKit {
    /// Strike the drum mapped to `note` (MIDI int or name). `velocity` is 0..1.
    #[pyo3(signature = (note, velocity=1.0))]
    fn note_on(&self, note: &Bound<'_, PyAny>, velocity: f32) -> PyResult<()> {
        let note = to_note(note)?;
        if let Some(kit) = lock(&self.shared).get_mut::<CoreDrumKit>(self.id) {
            kit.note_on(note, velocity);
        }
        Ok(())
    }
}

/// An adaptive-music bed: intensity-driven stems plus one-shot stingers.
#[pyclass]
struct AdaptiveMusic {
    shared: Shared,
    id: SourceId,
    sample_rate: u32,
}

#[pymethods]
impl AdaptiveMusic {
    /// Add a looping stem (SoundDoc JSON) that fades in once intensity reaches
    /// `fade_in_at` (`0.0` = always on).
    #[pyo3(signature = (doc_json, fade_in_at=0.0))]
    fn add_layer(&self, doc_json: &str, fade_in_at: f32) -> PyResult<()> {
        let doc = parse_doc(doc_json, self.sample_rate)?;
        let layer = LoopBuffer::from_doc(&doc);
        if let Some(music) = lock(&self.shared).get_mut::<CoreAdaptive>(self.id) {
            music.add_layer(layer, fade_in_at);
        }
        Ok(())
    }

    /// Set the intensity, 0..1 — stems cross-fade toward their new levels.
    fn set_intensity(&self, x: f32) {
        if let Some(music) = lock(&self.shared).get_mut::<CoreAdaptive>(self.id) {
            music.set_intensity(x);
        }
    }

    /// Fire a one-shot stinger (SoundDoc JSON) over the bed.
    fn stinger(&self, doc_json: &str) -> PyResult<()> {
        let doc = parse_doc(doc_json, self.sample_rate)?;
        if let Some(music) = lock(&self.shared).get_mut::<CoreAdaptive>(self.id) {
            music.stinger(&doc);
        }
        Ok(())
    }
}

/// A loaded SFX patch: trigger one-shot instances with per-trigger parameters.
#[pyclass]
struct PatchVoice {
    shared: Shared,
    sfx: SourceId,
    patch: PatchId,
}

#[pymethods]
impl PatchVoice {
    /// Play a one-shot instance. Named keyword arguments set patch parameters for
    /// this trigger (unknown names are ignored); omitted parameters keep their
    /// defaults.
    #[pyo3(signature = (**params))]
    fn trigger(&self, params: Option<BTreeMap<String, f32>>) -> PyResult<()> {
        let mut guard = lock(&self.shared);
        let Some(engine) = guard.get_mut::<CoreEngine>(self.sfx) else {
            return Ok(());
        };
        let handle = engine.play(self.patch);
        if let Some(values) = params {
            for (name, value) in values {
                if let Some(param) = engine.param(self.patch, &name) {
                    engine.set_param(handle, param, value, Tween::IMMEDIATE);
                }
            }
        }
        Ok(())
    }
}

/// Register the live-stream classes on the module.
pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<Engine>()?;
    m.add_class::<Instrument>()?;
    m.add_class::<DrumKit>()?;
    m.add_class::<AdaptiveMusic>()?;
    m.add_class::<PatchVoice>()?;
    Ok(())
}
