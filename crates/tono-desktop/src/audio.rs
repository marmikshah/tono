//! Native real-time audio: the pattern **deck** — a looping [`Player`] with a
//! click-free crossfade on every document swap, so a grid edit lands on the
//! next audio block without restarting the loop or popping.
//!
//! `cpal::Stream` is `!Send`, so it can't live in shared (Tauri) state.
//! [`spawn`] builds the stream on a dedicated thread that owns it for the
//! process's life and hands back an [`AudioHandle`] — an `Arc<Mutex<Deck>>`,
//! which **is** `Send + Sync`. Rendering a swapped-in document happens on the
//! *caller's* thread; the lock is held only to move the pre-rendered player
//! in, and the audio callback only `try_lock`s, so an edit never blocks audio
//! (it drops at most one block).

use std::sync::mpsc;
use std::sync::{Arc, Mutex};

use anyhow::{Result, anyhow};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use tono_core::dsl::SoundDoc;
use tono_core::player::Player;

/// Doc-swap crossfade length (~20 ms) — long enough to declick, short enough
/// that an edit feels instant.
const SWAP_FADE_SECS: f32 = 0.02;

/// A transport action, parsed loudly at the Tauri boundary — a frontend typo
/// must error, not silently no-op.
pub enum TransportAction {
    /// Start/resume playback.
    Play,
    /// Freeze in place.
    Pause,
    /// Freeze and rewind.
    Stop,
}

impl std::str::FromStr for TransportAction {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, String> {
        match s {
            "play" => Ok(TransportAction::Play),
            "pause" => Ok(TransportAction::Pause),
            "stop" => Ok(TransportAction::Stop),
            other => Err(format!("unknown transport action: {other}")),
        }
    }
}

/// The playing state behind the mutex: the current loop, plus the outgoing
/// loop during a swap crossfade.
struct Deck {
    current: Option<Player>,
    /// `(player, remaining fade frames, total fade frames)` while a swapped-out
    /// loop is still ramping down.
    outgoing: Option<(Player, u32, u32)>,
    /// The transport: when false the callback writes silence and the play
    /// heads freeze (pause). Stop additionally rewinds.
    playing: bool,
}

/// A `Send + Sync` control handle to the running audio deck.
pub struct AudioHandle {
    deck: Arc<Mutex<Deck>>,
    device_sr: u32,
}

impl AudioHandle {
    /// Swap the loop for `doc` (or silence for `None`), preserving the play
    /// position modulo the new length and crossfading the old audio out. The
    /// render happens here, on the caller's thread — the audio thread only
    /// ever sees a finished buffer.
    pub fn set_doc(&self, doc: Option<SoundDoc>) {
        let fresh = doc.map(|mut d| {
            d.sample_rate = self.device_sr;
            let mut p = Player::new(d);
            p.looping = true;
            p
        });
        let fade = ((SWAP_FADE_SECS * self.device_sr as f32) as u32).max(1);
        let mut deck = self.deck.lock().unwrap_or_else(|p| p.into_inner());
        let old = deck.current.take();
        match (fresh, old) {
            (Some(mut new), Some(old)) => {
                if new.frames() > 0 {
                    new.seek(old.position() % new.frames());
                }
                new.play();
                deck.outgoing = Some((old, fade, fade));
                deck.current = Some(new);
            }
            (Some(mut new), None) => {
                new.play();
                deck.current = Some(new);
            }
            (None, old) => {
                // The grid went silent: fade whatever was playing out.
                if let Some(old) = old {
                    deck.outgoing = Some((old, fade, fade));
                }
            }
        }
    }

    /// Drive the transport (see [`TransportAction`]).
    pub fn transport(&self, action: TransportAction) {
        let mut deck = self.deck.lock().unwrap_or_else(|p| p.into_inner());
        match action {
            TransportAction::Play => deck.playing = true,
            TransportAction::Pause => deck.playing = false,
            TransportAction::Stop => {
                deck.playing = false;
                if let Some(p) = deck.current.as_mut() {
                    p.seek(0);
                }
            }
        }
    }

    /// `(playing, position frames, loop frames)` for the UI's playhead.
    pub fn playhead(&self) -> (bool, usize, usize) {
        let deck = self.deck.lock().unwrap_or_else(|p| p.into_inner());
        let (pos, len) = deck
            .current
            .as_ref()
            .map(|p| (p.position(), p.frames()))
            .unwrap_or((0, 0));
        (deck.playing, pos, len)
    }
}

/// Open the default output device and start the (paused, empty) deck. The
/// `cpal::Stream` is owned by a dedicated thread for the process's life.
pub fn spawn() -> Result<AudioHandle> {
    let (tx, rx) = mpsc::channel();
    std::thread::Builder::new()
        .name("tono-audio".into())
        .spawn(move || match build_stream() {
            Ok((stream, handle)) => {
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
fn build_stream() -> Result<(cpal::Stream, AudioHandle)> {
    let host = cpal::default_host();
    let device = host
        .default_output_device()
        .ok_or_else(|| anyhow!("no default audio output device"))?;
    let supported = device.default_output_config()?;
    let device_sr = supported.sample_rate().0;
    let channels = supported.channels() as usize;
    let sample_format = supported.sample_format();
    let config: cpal::StreamConfig = supported.into();

    let deck = Arc::new(Mutex::new(Deck {
        current: None,
        outgoing: None,
        playing: false,
    }));
    let cb_deck = deck.clone();
    let mut now = Vec::<f32>::new();
    let mut old = Vec::<f32>::new();
    let err_fn = |e| eprintln!("tono audio stream error: {e}");

    let stream = match sample_format {
        cpal::SampleFormat::F32 => device.build_output_stream(
            &config,
            move |data: &mut [f32], _| {
                // Never unwind into cpal's C callback (UB): contain any panic in
                // the mix path and fall back to silence.
                let guarded = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    mix(&cb_deck, data, channels, &mut now, &mut old)
                }));
                if guarded.is_err() {
                    data.fill(0.0);
                }
            },
            err_fn,
            None,
        )?,
        other => {
            return Err(anyhow!(
                "device sample format {other:?} unsupported (the deck is f32)"
            ));
        }
    };
    stream.play()?;
    Ok((stream, AudioHandle { deck, device_sr }))
}

/// Audio-callback body: serve the current loop, mixing the outgoing one down
/// its swap fade. Never blocks — a held control-thread lock yields silence for
/// one block. `now`/`old` are reused scratch.
fn mix(
    deck: &Arc<Mutex<Deck>>,
    data: &mut [f32],
    channels: usize,
    now: &mut Vec<f32>,
    old: &mut Vec<f32>,
) {
    let frames = data.len() / channels.max(1);
    now.resize(frames * 2, 0.0);
    now.fill(0.0);

    if let Ok(mut deck) = deck.try_lock()
        && deck.playing
    {
        if let Some(p) = deck.current.as_mut() {
            p.fill(now);
        }
        if let Some((out_player, remaining, total)) = deck.outgoing.as_mut() {
            old.resize(frames * 2, 0.0);
            out_player.fill(old);
            let total = *total as f32;
            for f in 0..frames {
                let w = *remaining as f32 / total; // outgoing weight, 1 → 0
                now[f * 2] = now[f * 2] * (1.0 - w) + old[f * 2] * w;
                now[f * 2 + 1] = now[f * 2 + 1] * (1.0 - w) + old[f * 2 + 1] * w;
                *remaining = remaining.saturating_sub(1);
            }
            if *remaining == 0 {
                deck.outgoing = None;
            }
        }
    }

    // Hard safety clamp before the device write (the audition path can spike
    // mid-edit), then the shared channel spread.
    for s in now[..frames * 2].iter_mut() {
        *s = s.clamp(-1.0, 1.0);
    }
    tono_core::runtime::write_interleaved(data, channels, &now[..frames * 2]);
}
