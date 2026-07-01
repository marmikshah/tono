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

pub mod adaptive;
#[cfg(feature = "analysis")]
pub mod analysis;
pub mod drumkit;
pub mod dsl;
pub mod dsp;
pub mod edit;
pub mod instrument;
pub mod patch;
pub mod presets;
pub mod render;
#[cfg(feature = "analysis")]
pub mod review;
pub mod runtime;
pub mod song;
pub mod stream;
pub mod streaming;
pub mod vary;
pub mod voice;
