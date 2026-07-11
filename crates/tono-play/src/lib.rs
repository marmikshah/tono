//! tono-play — the programmatic playground.
//!
//! Build a sound or an instrument in a few lines of Rust and hear it through the
//! default output device:
//!
//! ```no_run
//! use tono_play::{play_doc, Speaker, device_sample_rate};
//! use tono_core::instrument::{Instrument, InstrumentDesign, Note};
//! # fn demo(doc: &tono_core::dsl::SoundDoc, patch: tono_core::patch::Patch) -> anyhow::Result<()> {
//! play_doc(doc, 0.6)?;                                   // hear a sound
//! let sr = device_sample_rate()?;
//! let inst = Instrument::new(InstrumentDesign::new(patch), sr)?;
//! let speaker = Speaker::open(inst)?;                    // keep it playing
//! speaker.control(|i| { i.note_on(Note::C4, 0.9); });    // drive it live
//! # Ok(()) }
//! ```
//!
//! Sources render at the device's sample rate — build your `Engine`/`Instrument`
//! with [`device_sample_rate`]. This uses a `Mutex` around the source (fine for a
//! playground/prototype); a shipping game wants the wait-free
//! [`Engine::split`](tono_core::runtime::Engine::split) seam instead.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use tono_core::dsl::SoundDoc;
use tono_core::runtime::{AudioSource, Engine, StreamSource};

/// The default output device's sample rate. Build your `Engine` / `Instrument`
/// with this so it renders at the rate the speaker consumes.
pub fn device_sample_rate() -> anyhow::Result<u32> {
    let host = cpal::default_host();
    let device = host
        .default_output_device()
        .ok_or_else(|| anyhow::anyhow!("no default output device"))?;
    Ok(device.default_output_config()?.sample_rate().0)
}

/// A live audio output: a stream feeding an [`AudioSource`] until dropped. Drive
/// the source live with [`control`](Self::control).
pub struct Speaker<S: AudioSource + Send> {
    source: Arc<Mutex<S>>,
    _stream: cpal::Stream,
    sample_rate: u32,
}

impl<S: AudioSource + Send + 'static> Speaker<S> {
    /// Open the default output device and start streaming `source` (rendered at
    /// the device sample rate — build the source with [`device_sample_rate`]).
    pub fn open(source: S) -> anyhow::Result<Speaker<S>> {
        let host = cpal::default_host();
        let device = host
            .default_output_device()
            .ok_or_else(|| anyhow::anyhow!("no default output device"))?;
        let config = device.default_output_config()?;
        let sample_rate = config.sample_rate().0;
        let channels = config.channels() as usize;
        let sample_format = config.sample_format();
        let stream_config: cpal::StreamConfig = config.into();

        let source = Arc::new(Mutex::new(source));
        let cb = source.clone();
        let mut scratch: Vec<f32> = Vec::new();
        let on_err = move |e| eprintln!("tono-play: output stream error: {e}");

        // Only f32 output is supported (the default on modern CoreAudio / WASAPI /
        // ALSA); other formats error with a clear message.
        let stream = match sample_format {
            cpal::SampleFormat::F32 => device.build_output_stream(
                &stream_config,
                move |data: &mut [f32], _: &cpal::OutputCallbackInfo| {
                    let chans = channels.max(1);
                    // Two audio-thread rules, both load-bearing on the C callback:
                    //   1. Never block — `try_lock`, not `lock`; if `control` holds
                    //      the source, output one silent block rather than stall.
                    //   2. Never unwind into cpal's C frame (UB) — contain any
                    //      panic in the render path and fall back to silence.
                    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        let frames = data.len() / chans;
                        if scratch.len() < frames * 2 {
                            scratch.resize(frames * 2, 0.0);
                        }
                        let st = &mut scratch[..frames * 2];
                        let locked = match cb.try_lock() {
                            Ok(s) => Some(s),
                            // A prior panic poisoned it; the source is plain audio
                            // state, so recover and keep playing.
                            Err(std::sync::TryLockError::Poisoned(p)) => Some(p.into_inner()),
                            Err(std::sync::TryLockError::WouldBlock) => None,
                        };
                        if let Some(mut s) = locked {
                            s.fill(st);
                            drop(s);
                            tono_core::runtime::write_interleaved(data, chans, st);
                        } else {
                            data.fill(0.0);
                        }
                    }));
                    if result.is_err() {
                        data.fill(0.0);
                    }
                },
                on_err,
                None,
            )?,
            other => {
                anyhow::bail!("unsupported output sample format {other:?} (tono-play needs f32)")
            }
        };
        stream.play()?;
        Ok(Speaker {
            source,
            _stream: stream,
            sample_rate,
        })
    }

    /// The device sample rate the source renders at.
    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    /// Drive the playing source (e.g. `instrument.note_on(...)`). Holds the
    /// source lock while `f` runs; the audio thread `try_lock`s, so a long `f`
    /// costs at most a silent block rather than a stall — fine for a
    /// playground/prototype.
    pub fn control<R>(&self, f: impl FnOnce(&mut S) -> R) -> R {
        // Recover from a poisoned lock (a panic on another thread) — the
        // source is plain audio state and controlling it must not cascade
        // the panic into the caller.
        f(&mut self.source.lock().unwrap_or_else(|p| p.into_inner()))
    }
}

/// Play `source` through the speakers for `secs` seconds (blocking).
pub fn play<S: AudioSource + Send + 'static>(source: S, secs: f32) -> anyhow::Result<()> {
    let speaker = Speaker::open(source)?;
    std::thread::sleep(Duration::from_secs_f32(secs.max(0.0)));
    drop(speaker);
    Ok(())
}

/// Play a [`SoundDoc`] for `secs` seconds — streamed if it's in the streamable
/// subset, else buffered — one call to hear a sound you built in code.
pub fn play_doc(doc: &SoundDoc, secs: f32) -> anyhow::Result<()> {
    // Validate up front: a malformed doc should error loudly here, not play
    // silence (the classic case is the `env`-fields-nested-under-"adsr"
    // serde-flatten footgun, which renders as an all-zero envelope).
    doc.validate().map_err(|e| anyhow::anyhow!(e))?;
    let sr = device_sample_rate()?;
    let mut doc = doc.clone();
    doc.sample_rate = sr;
    if let Some(src) = StreamSource::from_doc(&doc) {
        play(src, secs)
    } else {
        let mut engine = Engine::new(sr);
        let patch = engine.load(&doc);
        engine.play_looping(patch);
        play(engine, secs)
    }
}
