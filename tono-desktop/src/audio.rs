//! Native real-time audio host: a [`Player`] (patch preview) and an
//! [`Instrument`] (the *currently designed* sound, played polyphonically from the
//! computer keyboard or MIDI) summed through one `cpal` output stream.
//!
//! `cpal::Stream` is `!Send`, so it can't live in shared (Tauri) state. [`spawn`]
//! builds the stream on a dedicated thread that owns it for the process's life,
//! and hands back an [`AudioHandle`] — shared `Arc<Mutex<…>>` controls, which
//! **are** `Send + Sync`. The audio callback only reads via `try_lock`, so a
//! control-thread edit never blocks audio (it drops at most one block). Everything
//! renders at the device sample rate so playback/pitch are correct.

use std::sync::mpsc;
use std::sync::{Arc, Mutex};

use anyhow::{Result, anyhow};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use midir::{MidiInput, MidiInputConnection};
use tono_core::dsl::{Adsr, SoundDoc};
use tono_core::instrument::{Instrument, InstrumentDesign, Note};
use tono_core::patch::Patch;
use tono_core::runtime::AudioSource;
use tono_core::stream::Player;

/// Headroom so a fistful of held notes doesn't clip against the patch preview.
const KEYS_GAIN: f32 = 0.6;

fn default_amp() -> Adsr {
    Adsr {
        a: 0.01,
        d: 0.12,
        s: 0.6,
        r: 0.2,
        punch: 0.0,
    }
}

/// The keyboard voice — the currently-designed sound as a playable instrument —
/// plus the amp envelope and the doc it was built from (so it rebuilds when the
/// sound or envelope changes). `instrument` is `None` if the doc isn't streamable.
struct Keyboard {
    instrument: Option<Instrument>,
    amp: Adsr,
    doc: SoundDoc,
    sr: u32,
}

impl Keyboard {
    fn rebuild(&mut self) {
        let patch = Patch {
            doc: self.doc.clone(),
            params: Vec::new(),
        };
        let design = InstrumentDesign::new(patch).with_amp(self.amp);
        self.instrument = Instrument::new(design, self.sr).ok();
    }
}

/// A `Send + Sync` control handle to the running audio engine.
#[derive(Clone)]
pub struct AudioHandle {
    player: Arc<Mutex<Player>>,
    keys: Arc<Mutex<Keyboard>>,
    device_sr: u32,
}

impl AudioHandle {
    /// Swap in a new document: re-render the preview AND rebuild the keyboard
    /// instrument, so the keys now play the sound you just designed.
    pub fn set_doc(&self, mut doc: SoundDoc) {
        doc.sample_rate = self.device_sr;
        if let Ok(mut p) = self.player.lock() {
            p.set_doc(doc.clone());
        }
        if let Ok(mut k) = self.keys.lock() {
            k.doc = doc;
            k.rebuild();
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

    /// Set the keyboard's amplitude envelope (rebuilds the instrument).
    pub fn set_amp(&self, env: Adsr) {
        if let Ok(mut k) = self.keys.lock() {
            k.amp = env;
            k.rebuild();
        }
    }

    /// Strike a live note (MIDI note number + velocity in `[0, 1]`).
    pub fn note_on(&self, note: u8, velocity: f32) {
        if let Ok(mut k) = self.keys.lock()
            && let Some(inst) = k.instrument.as_mut()
        {
            inst.note_on(Note(note), velocity);
        }
    }

    /// Release a live note.
    pub fn note_off(&self, note: u8) {
        if let Ok(mut k) = self.keys.lock()
            && let Some(inst) = k.instrument.as_mut()
        {
            inst.note_off(Note(note));
        }
    }

    /// The device sample rate everything renders at.
    pub fn device_sample_rate(&self) -> u32 {
        self.device_sr
    }
}

/// Open the default output device and start a paused real-time stream loaded
/// with `doc`. The `cpal::Stream` is owned by a dedicated thread for the
/// process's life; the returned handle controls the shared player + keyboard.
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
    let player = Arc::new(Mutex::new(Player::new(doc.clone())));
    let mut keyboard = Keyboard {
        instrument: None,
        amp: default_amp(),
        doc,
        sr: device_sr,
    };
    keyboard.rebuild();
    let keys = Arc::new(Mutex::new(keyboard));

    let cb_player = player.clone();
    let cb_keys = keys.clone();
    let mut preview = Vec::<f32>::new();
    let mut voice = Vec::<f32>::new();
    let err_fn = |e| eprintln!("tono audio stream error: {e}");

    let stream = match sample_format {
        cpal::SampleFormat::F32 => device.build_output_stream(
            &config,
            move |data: &mut [f32], _| {
                mix(
                    &cb_player,
                    &cb_keys,
                    data,
                    channels,
                    &mut preview,
                    &mut voice,
                )
            },
            err_fn,
            None,
        )?,
        other => {
            return Err(anyhow!(
                "device sample format {other:?} unsupported (audition is f32)"
            ));
        }
    };
    stream.play()?;
    Ok((
        stream,
        AudioHandle {
            player,
            keys,
            device_sr,
        },
    ))
}

/// Audio-callback body: sum the patch preview and the live keyboard instrument
/// (both interleaved stereo) into `data`. Never blocks — a held control-thread
/// lock yields silence for that source. `preview`/`voice` are reused scratch.
fn mix(
    player: &Arc<Mutex<Player>>,
    keys: &Arc<Mutex<Keyboard>>,
    data: &mut [f32],
    channels: usize,
    preview: &mut Vec<f32>,
    voice: &mut Vec<f32>,
) {
    let frames = data.len() / channels.max(1);
    preview.resize(frames * 2, 0.0);
    match player.try_lock() {
        Ok(mut p) => {
            p.fill(preview);
        }
        Err(_) => preview.iter_mut().for_each(|x| *x = 0.0),
    }
    voice.resize(frames * 2, 0.0);
    match keys.try_lock() {
        Ok(mut k) => match k.instrument.as_mut() {
            Some(inst) => {
                inst.fill(voice);
            }
            None => voice.iter_mut().for_each(|x| *x = 0.0),
        },
        Err(_) => voice.iter_mut().for_each(|x| *x = 0.0),
    }
    for f in 0..frames {
        let l = (preview[f * 2] + voice[f * 2] * KEYS_GAIN).clamp(-1.0, 1.0);
        let r = (preview[f * 2 + 1] + voice[f * 2 + 1] * KEYS_GAIN).clamp(-1.0, 1.0);
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

/// Connect every MIDI input port present at startup to the live keyboard.
/// Failures (no MIDI backend, no devices) degrade gracefully to "keyboard only".
/// Devices plugged in later aren't hot-detected.
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

/// Translate a raw MIDI channel message into keyboard note events. Note-on with
/// zero velocity is treated as note-off (running-status convention); CC64 drives
/// the sustain pedal.
fn midi_message(msg: &[u8], handle: &AudioHandle) {
    if msg.len() < 3 {
        return;
    }
    let note = msg[1];
    match (msg[0] & 0xF0, msg[2]) {
        (0x90, vel) if vel > 0 => handle.note_on(note, vel as f32 / 127.0),
        (0x80, _) | (0x90, 0) => handle.note_off(note),
        (0xB0, val) if msg[1] == 64 => {
            if let Ok(mut k) = handle.keys.lock()
                && let Some(inst) = k.instrument.as_mut()
            {
                inst.set_sustain(val >= 64);
            }
        }
        _ => {}
    }
}
