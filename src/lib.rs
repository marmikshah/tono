//! tono — a deterministic sound engine.
//!
//! The pure, headless engine — the `SoundDoc` graph DSL, DSP, the deterministic
//! renderer, the byte-identical streaming renderer, analysis/critique, the
//! instrument / song / drum-kit / adaptive-music layers — lives in the
//! [`tono_core`] crate and is re-exported here.
//!
//! This crate is the thin **shell** around it: audio-file encoders, the analysis
//! image writer, MIDI export, and the `tono` command-line tool that renders a
//! `SoundDoc` to audio + feedback images (see `src/main.rs`).

pub use tono_core::{analysis, dsl, dsp, edit, render, review, vary};

pub mod audio;
pub mod imaging;
pub mod midi;
