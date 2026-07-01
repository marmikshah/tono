//! Tono core — the pure, headless audio engine.
//!
//! This crate is the deterministic heart of tono with **no I/O, no MCP, and
//! no transport**: the symbolic synthesis-graph data model ([`dsl`]), the DSP
//! primitives ([`dsp`]), the renderer ([`render`]), the analysis/critique
//! feedback ([`analysis`], [`review`]), and the pure graph transforms
//! ([`edit`], [`vary`]). Rendering is a pure function of
//! `(graph, seed, sample_rate)` → byte-identical audio.
//!
//! The lean build is pure compute (serde only); the heavy deps are optional,
//! behind features (both on by default): `analysis` pulls in rustfft + image for
//! [`analysis`]/[`review`], and `sampler` pulls in rustysynth for the SoundFont
//! sampler instrument. So the same core compiles to a native binary, a WASM
//! playground, or a lean in-engine runtime. The MCP server, file encoders,
//! persistence, and daemon live in the `tono` shell crate that depends on this one.

#[cfg(feature = "analysis")]
pub mod analysis;
pub mod dsl;
pub mod dsp;
pub mod edit;
pub mod patch;
pub mod render;
#[cfg(feature = "analysis")]
pub mod review;
pub mod stream;
pub mod vary;
pub mod voice;
