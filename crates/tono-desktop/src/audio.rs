//! Native real-time audio: the pattern **deck** — a looping [`Player`] with a
//! click-free crossfade on every document swap, so a grid edit lands on the
//! next audio block without restarting the loop or popping.
//!
//! The cpal plumbing (device open, f32 gate, panic containment, channel
//! spread) is [`tono_play::Speaker`]'s job — one shared shim across the native
//! faces. A `cpal::Stream` is `!Send`, so [`spawn`] parks the `Speaker` on a
//! dedicated thread for the process's life and hands back an [`AudioHandle`]
//! over [`Speaker::shared`]. Rendering a swapped-in document happens on the
//! *caller's* thread; the lock is held only to move the pre-rendered player
//! in, and the audio callback only `try_lock`s, so an edit never blocks audio
//! (it drops at most one block).

use std::sync::mpsc;
use std::sync::{Arc, Mutex};

use anyhow::{Result, anyhow};
use tono_core::dsl::SoundDoc;
use tono_core::player::Player;
use tono_core::runtime::AudioSource;
use tono_play::Speaker;

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
    /// Reused scratch for the outgoing loop during a crossfade.
    outgoing_scratch: Vec<f32>,
}

impl AudioSource for Deck {
    /// Serve the current loop, mixing the outgoing one down its swap fade,
    /// then hard-clamp (the audition path can spike mid-edit). Paused: silence
    /// without advancing any play head.
    fn fill(&mut self, out: &mut [f32]) -> usize {
        let frames = out.len() / 2;
        out.fill(0.0);
        if !self.playing {
            return frames;
        }
        if let Some(p) = self.current.as_mut() {
            p.fill(out);
        }
        if let Some((out_player, remaining, total)) = self.outgoing.as_mut() {
            if self.outgoing_scratch.len() < frames * 2 {
                self.outgoing_scratch.resize(frames * 2, 0.0);
            }
            let old = &mut self.outgoing_scratch[..frames * 2];
            out_player.fill(old);
            let total = *total as f32;
            for f in 0..frames {
                let w = *remaining as f32 / total; // outgoing weight, 1 → 0
                out[f * 2] = out[f * 2] * (1.0 - w) + old[f * 2] * w;
                out[f * 2 + 1] = out[f * 2 + 1] * (1.0 - w) + old[f * 2 + 1] * w;
                *remaining = remaining.saturating_sub(1);
            }
            if *remaining == 0 {
                self.outgoing = None;
            }
        }
        for s in out[..frames * 2].iter_mut() {
            *s = s.clamp(-1.0, 1.0);
        }
        frames
    }
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
/// (`!Send`) `Speaker` is parked on a dedicated thread for the process's life;
/// the handle controls the deck through [`Speaker::shared`].
pub fn spawn() -> Result<AudioHandle> {
    let (tx, rx) = mpsc::channel();
    std::thread::Builder::new()
        .name("tono-audio".into())
        .spawn(move || {
            let deck = Deck {
                current: None,
                outgoing: None,
                playing: false,
                outgoing_scratch: Vec::new(),
            };
            match Speaker::open(deck) {
                Ok(speaker) => {
                    tx.send(Ok(AudioHandle {
                        deck: speaker.shared(),
                        device_sr: speaker.sample_rate(),
                    }))
                    .ok();
                    let _speaker = speaker;
                    loop {
                        std::thread::park();
                    }
                }
                Err(e) => {
                    tx.send(Err(e.to_string())).ok();
                }
            }
        })?;
    rx.recv()
        .map_err(|_| anyhow!("audio thread exited before starting"))?
        .map_err(|e| anyhow!(e))
}
