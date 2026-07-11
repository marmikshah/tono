//! Tono core — the pure, headless audio engine.
//!
//! This crate is the deterministic heart of tono with **no I/O and no
//! transport**: the symbolic synthesis-graph data model ([`dsl`]), the DSP
//! primitives ([`dsp`]), the renderer ([`render`]), the analysis/critique
//! feedback ([`analysis`], [`review`]), and the pure graph transforms
//! ([`edit`], [`vary`]). Rendering is a pure function of
//! `(graph, seed, sample_rate)` → byte-identical audio.
//!
//! The lean build is pure compute (serde only); the heavy deps are optional,
//! behind features (both on by default): `analysis` pulls in rustfft + image for
//! [`analysis`]/[`review`], and `sampler` pulls in rustysynth for the SoundFont
//! sampler instrument. So the same core compiles to a native binary, a WASM
//! playground, or a lean in-engine runtime. The `tono render` CLI, audio-file
//! encoders, and MIDI export live in the `tono` shell crate that depends on this one.

#![warn(missing_docs)]

pub mod adaptive;
#[cfg(feature = "analysis")]
pub mod analysis;
pub mod catalog;
pub mod drumkit;
pub mod dsl;
pub mod dsp;
pub mod edit;
pub mod instrument;
pub mod patch;
pub mod player;
pub mod presets;
pub mod render;
#[cfg(feature = "analysis")]
pub mod review;
pub mod runtime;
pub mod song;
pub mod streaming;
pub mod vary;
pub mod voice;

/// Renamed to [`player`] — `stream` (the buffer-backed audition `Player`) sat
/// one suffix away from [`streaming`] (the per-sample block renderer), and the
/// pair was a reliable source of confusion.
#[deprecated(since = "1.6.0", note = "renamed to `player`")]
pub use player as stream;

/// The workhorse names in one import: `use tono_core::prelude::*;` covers the
/// primary flow (author a doc or a [`song::Song`], render it, analyze it, play
/// it) without hunting across the crate's nineteen modules.
pub mod prelude {
    #[cfg(feature = "analysis")]
    pub use crate::analysis::{Analysis, stats, stats_stereo};
    pub use crate::catalog::{
        Bass, Drums, ElectricPiano, GrandPiano, Guitar, Organ, Strings, Voice,
    };
    pub use crate::dsl::{Adsr, ENGINE_VERSION, Node, SeqNote, SeqWave, SoundDoc, Value};
    pub use crate::instrument::{Instrument, InstrumentDesign, Note};
    pub use crate::patch::Patch;
    pub use crate::render::{RenderProduct, render, render_product};
    pub use crate::runtime::{AudioSource, Engine, Mixer, StreamSource, Tween};
    pub use crate::song::{Song, note, note_vel};
}
