//! Native real-time audio host: drives a [`Player`] from a `cpal` output stream.
//!
//! The control side (re-rendering on edit) runs off the audio thread; the audio
//! callback only reads the rendered buffers and advances the play head — and
//! uses `try_lock`, so a re-render in progress drops at most one block rather
//! than ever blocking the audio thread. The document is rendered at the device
//! sample rate so playback speed is correct regardless of the document's own
//! rate; an offline bounce still uses the document's rate (the determinism
//! invariant is about the graph, not the audition rate).

use std::sync::{Arc, Mutex};

use anyhow::{Result, anyhow};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use sonarium_core::dsl::SoundDoc;
use sonarium_core::stream::Player;

/// A running native audio engine. Hold it alive to keep the stream open.
pub struct AudioEngine {
    player: Arc<Mutex<Player>>,
    _stream: cpal::Stream,
    device_sr: u32,
}

impl AudioEngine {
    /// Open the default output device and start a paused stream loaded with
    /// `doc` (rendered at the device's sample rate). Call [`AudioEngine::play`]
    /// to start the play head.
    pub fn new(mut doc: SoundDoc) -> Result<Self> {
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
        Ok(Self {
            player,
            _stream: stream,
            device_sr,
        })
    }

    /// Swap in a new document (re-rendered at the device rate) without stopping
    /// playback — the live-edit path.
    // The transport/live-edit API below is exercised by the Tauri command layer.
    #[allow(dead_code)]
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
    #[allow(dead_code)]
    pub fn stop(&self) {
        if let Ok(mut p) = self.player.lock() {
            p.stop();
        }
    }

    /// Toggle looping.
    #[allow(dead_code)]
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
    // Non-stereo device: fill a stereo scratch then spread/downmix per frame.
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
