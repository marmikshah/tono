//! runtime — the embeddable real-time control surface over the deterministic engine.
//!
//! The idiomatic library API a game (or any host) drives: [`Engine::load`] a
//! [`SoundDoc`](crate::dsl::SoundDoc) (or [`Engine::load_patch`] a [`Patch`](crate::patch::Patch) with named parameters) as
//! a reusable **resource**, [`Engine::play`] as many independent **instances** as
//! you like, and control each by its [`InstanceHandle`] with [`Tween`]-smoothed
//! setters. Host output adapters (cpal, an AudioWorklet, a Bevy source) target
//! the [`AudioSource`] trait, so they never depend on a concrete engine type.
//!
//! Backed today by the deterministic buffer renderer ([`crate::player::Player`]),
//! which keeps the mix **byte-identical to an offline bounce**. Instance master
//! controls (gain / pan / stop) apply live per block; parameter and layer-gain
//! changes ([`Engine::set_param`] / [`Engine::set_layer_gain`]) re-render the
//! instance and **crossfade** for a click-free swap — control-rate today, and
//! sample-accurate once the stateful streaming renderer lands behind this same
//! seam. Multi-threaded real-time use goes through [`Engine::split`].
//!
//! # Adapters
//!
//! A host output is a thin shim over [`AudioSource`] + [`Engine::split`]. cpal:
//!
//! ```ignore
//! let (mut control, mut audio) = Engine::new(sr).split(2048);
//! let stream = device.build_output_stream(
//!     &config,
//!     move |out: &mut [f32], _| { audio.fill(out); }, // audio thread drains the ring
//!     err_fn, None,
//! )?;
//! stream.play()?;
//! // On a control thread: loop { control.pump(1024); std::thread::sleep(dt); }
//! ```
//!
//! A Bevy `Decodable` / rodio `Source` wraps the same [`Renderer`]; an
//! AudioWorklet calls [`AudioSource::fill`] on each 128-frame quantum.

mod engine;
mod mixer;
mod ring;
mod source;

pub use engine::{Engine, InstanceHandle, LayerId, ParamId, PatchId, Priority, Tween};
pub use mixer::{BusId, Mixer, MixerError, SourceId};
pub use ring::{Controller, Pump, Renderer, spsc};
pub use source::{AudioSource, StreamSource, write_interleaved};

#[cfg(test)]
mod tests;
