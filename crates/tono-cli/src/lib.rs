//! File and command-line adapters for tono's audio engine.
//!
//! This crate supplies audio-file encoding, analysis images, MIDI import and
//! export, and compilation inspection. Graphs, DSP, songs,
//! instruments and the live runtime belong to [`tono_core`].

#![warn(missing_docs)]

pub mod audio;
pub mod audition;
pub mod compile;
pub mod imaging;
#[cfg(feature = "midi")]
pub mod midi;
#[cfg(feature = "play")]
pub mod play;
