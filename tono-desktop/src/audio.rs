//! Native real-time audio host: drives a [`Player`] (patch preview) and a
//! [`PolySynth`] (live keyboard/MIDI notes) from one `cpal` output stream.
//!
//! `cpal::Stream` is `!Send`, so it can't live in shared (Tauri) state. [`spawn`]
//! builds the stream on a dedicated thread that owns it for the process's life,
//! and hands back an [`AudioHandle`] — shared `Arc<Mutex<…>>` controls, which
//! **are** `Send + Sync`. The audio callback only reads via `try_lock`, so a
//! control-thread edit never blocks audio (it drops at most one block). Rendering
//! happens at the device sample rate so playback/pitch are correct.

use std::sync::mpsc;
use std::sync::{Arc, Mutex};

use anyhow::{Result, anyhow};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use midir::{MidiInput, MidiInputConnection};
use tono_core::dsl::{Adsr, Shape, SoundDoc};
use tono_core::stream::Player;
use tono_core::voice::PolySynth;

const MAX_VOICES: usize = 16;
/// Headroom so a fistful of held notes doesn't clip against the patch preview.
const SYNTH_GAIN: f32 = 0.28;

/// The live instrument plus a cheap fingerprint, so we only rebuild voices (and
/// cut held notes) when the instrument actually changes.
struct SynthSlot {
    synth: PolySynth,
    spec: (Shape, [f32; 4], f32),
}

fn default_instrument() -> (Shape, Adsr, f32) {
    (
        Shape::Sine,
        Adsr {
            a: 0.01,
            d: 0.1,
            s: 0.6,
            r: 0.2,
            punch: 0.0,
        },
        0.5,
    )
}

fn spec_of(shape: Shape, env: &Adsr, duty: f32) -> (Shape, [f32; 4], f32) {
    (shape, [env.a, env.d, env.s, env.r], duty)
}

/// A `Send + Sync` control handle to the running audio engine.
#[derive(Clone)]
pub struct AudioHandle {
    player: Arc<Mutex<Player>>,
    synth: Arc<Mutex<SynthSlot>>,
    device_sr: u32,
}

impl AudioHandle {
    /// Swap in a new document for the patch preview (re-rendered at the device
    /// rate) without stopping playback.
    pub fn set_doc(&self, mut doc: SoundDoc) {
        doc.sample_rate = self.device_sr;
        if let Ok(mut p) = self.player.lock() {
            p.set_doc(doc);
        }
    }

    /// Start the patch-preview play head.
    pub fn play(&self) {
        if let Ok(mut p) = self.player.lock() {
            p.play();
        }
    }

    /// Stop and rewind the patch preview.
    pub fn stop(&self) {
        if let Ok(mut p) = self.player.lock() {
            p.stop();
        }
    }

    /// Configure the live instrument (rebuilds voices only if it changed).
    pub fn set_instrument(&self, shape: Shape, env: Adsr, duty: f32) {
        if let Ok(mut slot) = self.synth.lock() {
            let spec = spec_of(shape, &env, duty);
            if slot.spec != spec {
                let mut synth = PolySynth::new(shape, &env, self.device_sr, MAX_VOICES);
                synth.set_duty(duty);
                slot.synth = synth;
                slot.spec = spec;
            }
        }
    }

    /// Strike a live note: `key` identifies it for `note_off`, `freq` is its Hz.
    pub fn note_on(&self, key: u32, freq: f32) {
        if let Ok(mut slot) = self.synth.lock() {
            slot.synth.note_on(key, freq);
        }
    }

    /// Release a live note.
    pub fn note_off(&self, key: u32) {
        if let Ok(mut slot) = self.synth.lock() {
            slot.synth.note_off(key);
        }
    }

    /// The device sample rate everything renders at.
    pub fn device_sample_rate(&self) -> u32 {
        self.device_sr
    }
}

/// Open the default output device and start a paused real-time stream loaded
/// with `doc`. The `cpal::Stream` is owned by a dedicated thread for the
/// process's life; the returned handle controls the shared player + synth.
pub fn spawn(doc: SoundDoc) -> Result<AudioHandle> {
    let (tx, rx) = mpsc::channel();
    std::thread::Builder::new()
        .name("tono-audio".into())
        .spawn(move || match build_stream(doc) {
            Ok((stream, handle)) => {
                let _midi = connect_midi(handle.clone()); // keep MIDI connections alive
                tx.send(Ok(handle)).ok();
                let _stream = stream;
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
    let (shape, env, duty) = default_instrument();
    let synth = Arc::new(Mutex::new(SynthSlot {
        synth: PolySynth::new(shape, &env, device_sr, MAX_VOICES),
        spec: spec_of(shape, &env, duty),
    }));

    let cb_player = player.clone();
    let cb_synth = synth.clone();
    let mut stereo = Vec::<f32>::new();
    let mut mono = Vec::<f32>::new();
    let err_fn = |e| eprintln!("tono audio stream error: {e}");

    let stream = match sample_format {
        cpal::SampleFormat::F32 => device.build_output_stream(
            &config,
            move |data: &mut [f32], _| {
                mix(
                    &cb_player,
                    &cb_synth,
                    data,
                    channels,
                    &mut stereo,
                    &mut mono,
                )
            },
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
    Ok((
        stream,
        AudioHandle {
            player,
            synth,
            device_sr,
        },
    ))
}

/// Audio-callback body: sum the patch preview (stereo) and the live synth (mono,
/// spread to both channels) into `data`. Never blocks — a held control-thread
/// lock yields silence for that source. `stereo`/`mono` are reused scratch.
fn mix(
    player: &Arc<Mutex<Player>>,
    synth: &Arc<Mutex<SynthSlot>>,
    data: &mut [f32],
    channels: usize,
    stereo: &mut Vec<f32>,
    mono: &mut Vec<f32>,
) {
    let frames = data.len() / channels.max(1);
    stereo.resize(frames * 2, 0.0);
    match player.try_lock() {
        Ok(mut p) => {
            p.fill(stereo);
        }
        Err(_) => stereo.iter_mut().for_each(|x| *x = 0.0),
    }
    mono.resize(frames, 0.0);
    match synth.try_lock() {
        Ok(mut s) => s.synth.process(mono),
        Err(_) => mono.iter_mut().for_each(|x| *x = 0.0),
    }
    for f in 0..frames {
        let m = mono[f] * SYNTH_GAIN;
        let l = (stereo[f * 2] + m).clamp(-1.0, 1.0);
        let r = (stereo[f * 2 + 1] + m).clamp(-1.0, 1.0);
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

/// Connect every MIDI input port present at startup to the live synth. Failures
/// (no MIDI backend, no devices) degrade gracefully to "keyboard only". Devices
/// plugged in later aren't hot-detected in v1.
fn connect_midi(handle: AudioHandle) -> Vec<MidiInputConnection<()>> {
    let mut conns = Vec::new();
    let Ok(scan) = MidiInput::new("tono-scan") else {
        return conns;
    };
    for port in scan.ports() {
        let Ok(input) = MidiInput::new("tono") else {
            continue;
        };
        let name = input.port_name(&port).unwrap_or_else(|_| "midi".into());
        let h = handle.clone();
        match input.connect(
            &port,
            "tono-in",
            move |_t, msg, _| midi_message(msg, &h),
            (),
        ) {
            Ok(conn) => {
                eprintln!("tono: MIDI connected — {name}");
                conns.push(conn);
            }
            Err(e) => eprintln!("tono: MIDI connect failed ({name}): {e}"),
        }
    }
    conns
}

/// Translate a raw MIDI channel message into synth note events. Note-on with
/// zero velocity is treated as note-off (running-status convention).
fn midi_message(msg: &[u8], handle: &AudioHandle) {
    if msg.len() < 3 {
        return;
    }
    let note = msg[1] as u32;
    let freq = 440.0 * 2f32.powf((note as f32 - 69.0) / 12.0);
    match (msg[0] & 0xF0, msg[2]) {
        (0x90, vel) if vel > 0 => handle.note_on(note, freq),
        (0x80, _) | (0x90, 0) => handle.note_off(note),
        _ => {}
    }
}
