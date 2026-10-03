//! File and command-line adapters for tono's audio engine.
//!
//! This crate supplies audio-file encoding, analysis images, MIDI import and
//! export, compilation inspection, matching and fitting. Graphs, DSP, songs,
//! instruments and the live runtime belong to [`tono_core`].

#![warn(missing_docs)]

pub mod audio;
pub mod audition;
pub mod compile;
pub mod diff;
pub mod fit;
pub mod imaging;
pub mod midi;
#[cfg(feature = "play")]
pub mod play;
pub mod review;
pub mod target;
