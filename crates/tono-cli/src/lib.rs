//! tono — a deterministic sound engine.
//!
//! The pure, headless engine — the `SoundDoc` graph DSL, DSP, the deterministic
//! renderer, the byte-identical streaming renderer, analysis/critique, the
//! instrument / song / drum-kit / adaptive-music layers — lives in the
//! [`tono_core`] crate; every one of its modules is re-exported here.
//!
//! This crate is the thin **shell** around it: audio-file encoders, the analysis
//! image writer, MIDI export, and the `tono` command-line tool that renders a
//! `SoundDoc` to audio + feedback images (see `src/main.rs`).

#![warn(missing_docs)]

pub use tono_core::{
    adaptive, analysis, catalog, diag, drumkit, dsl, dsp, edit, generate, instrument, patch,
    player, prelude, presets, program, render, runtime, song, streaming, units, vary,
};

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
