//! Native real-time audio host: drives a [`Player`] from a `cpal` output stream.
//!
//! `cpal::Stream` is `!Send`, so it can't live in shared (Tauri) state. Instead
//! [`spawn`] builds the stream on a dedicated thread that owns it for the
//! process's life, and hands back an [`AudioHandle`] — just `Arc<Mutex<Player>>`
//! plus the device rate, which **is** `Send + Sync`. The control side
//! (re-rendering on edit) runs on the caller's thread; the audio callback only
//! reads the rendered buffers via `try_lock`, so a re-render never blocks audio
//! (it drops at most one block). The document is rendered at the device sample
//! rate so playback speed is correct; an offline bounce still uses the
//! document's own rate (the determinism invariant is about the graph).

use std::sync::mpsc;
use std::sync::{Arc, Mutex};

use anyhow::{Result, anyhow};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use sonarium_core::dsl::SoundDoc;
use sonarium_core::stream::Player;

/// A `Send + Sync` control handle to the running audio engine. Cloning is cheap
/// (shared player); dropping it does not stop audio (the stream thread owns it).
#[derive(Clone)]
pub struct AudioHandle {
    player: Arc<Mutex<Player>>,
    device_sr: u32,
}

impl AudioHandle {
    /// Swap in a new document (re-rendered at the device rate) without stopping
    /// playback — the live-edit path.
    pub fn set_doc(&self, mut doc: SoundDoc) {
        doc.sample_rate = self.device_sr;
        if let Ok(mut p) = self.player.lock() {
            p.set_doc(doc);
        }
    }

    /// Start the play head from its current position.
    pub fn play(&self) {
        if let Ok(mut p) = self.player.lock() {
            p.play();
        }
    }

    /// Stop and rewind.
    pub fn stop(&self) {
        if let Ok(mut p) = self.player.lock() {
            p.stop();
        }
    }

    /// Toggle looping.
    #[allow(dead_code)] // wired by the Tauri command layer as transport grows.
    pub fn set_looping(&self, looping: bool) {
        if let Ok(mut p) = self.player.lock() {
            p.looping = looping;
        }
    }

    /// The device sample rate audition renders at.
    pub fn device_sample_rate(&self) -> u32 {
        self.device_sr
    }
}

/// Open the default output device and start a paused real-time stream loaded
/// with `doc`. The `cpal::Stream` is owned by a dedicated thread for the
/// process's life; the returned handle controls the shared player.
pub fn spawn(doc: SoundDoc) -> Result<AudioHandle> {
    let (tx, rx) = mpsc::channel();
    std::thread::Builder::new()
        .name("sonarium-audio".into())
        .spawn(move || match build_stream(doc) {
            Ok((stream, handle)) => {
                tx.send(Ok(handle)).ok();
                let _stream = stream; // keep the stream alive for the process
                loop {
                    std::thread::park();
                }
            }
            Err(e) => {
                tx.send(Err(e.to_string())).ok();
            }
        })?;
    rx.recv()
        .map_err(|_| anyhow!("audio thread exited before starting"))?
        .map_err(|e| anyhow!(e))
}

/// Build the cpal output stream + a control handle. Runs on the audio thread.
fn build_stream(mut doc: SoundDoc) -> Result<(cpal::Stream, AudioHandle)> {
    let host = cpal::default_host();
    let device = host
        .default_output_device()
        .ok_or_else(|| anyhow!("no default audio output device"))?;
    let supported = device.default_output_config()?;
    let device_sr = supported.sample_rate().0;
    let channels = supported.channels() as usize;
    let sample_format = supported.sample_format();
    let config: cpal::StreamConfig = supported.into();

    doc.sample_rate = device_sr;
    let player = Arc::new(Mutex::new(Player::new(doc)));
    let cb_player = player.clone();
    let err_fn = |e| eprintln!("sonarium audio stream error: {e}");

    let stream = match sample_format {
        cpal::SampleFormat::F32 => device.build_output_stream(
            &config,
            move |data: &mut [f32], _| fill_device(&cb_player, data, channels),
            err_fn,
            None,
        )?,
        other => {
            return Err(anyhow!(
                "device sample format {other:?} unsupported (v1 audition is f32)"
            ));
        }
    };
    stream.play()?;
    Ok((stream, AudioHandle { player, device_sr }))
}

/// Audio-callback body: fill `data` (interleaved, `channels`-wide) from the
/// player's stereo output. Never blocks — a held control-thread lock yields a
/// silent block instead.
fn fill_device(player: &Arc<Mutex<Player>>, data: &mut [f32], channels: usize) {
    if channels == 2 {
        match player.try_lock() {
            Ok(mut p) => {
                p.fill(data);
            }
            Err(_) => data.fill(0.0),
        }
        return;
    }
    let frames = data.len() / channels.max(1);
    let mut scratch = vec![0.0f32; frames * 2];
    match player.try_lock() {
        Ok(mut p) => {
            p.fill(&mut scratch);
        }
        Err(_) => scratch.fill(0.0),
    }
    for f in 0..frames {
        let (l, r) = (scratch[f * 2], scratch[f * 2 + 1]);
        let base = f * channels;
        if channels == 1 {
            data[base] = 0.5 * (l + r);
            continue;
        }
        data[base] = l;
        data[base + 1] = r;
        for c in 2..channels {
            data[base + c] = 0.0;
        }
    }
}
