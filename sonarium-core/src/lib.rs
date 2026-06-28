//! Sonarium core — the pure, headless audio engine.
//!
//! This crate is the deterministic heart of sonarium with **no I/O, no MCP, and
//! no transport**: the symbolic synthesis-graph data model ([`dsl`]), the DSP
//! primitives ([`dsp`]), the renderer ([`render`]), the analysis/critique
//! feedback ([`analysis`], [`review`]), and the pure graph transforms
//! ([`edit`], [`vary`]). Rendering is a pure function of
//! `(graph, seed, sample_rate)` → byte-identical audio.
//!
//! Everything here depends only on compute crates (serde, rustfft, image,
//! rustysynth), so the same core compiles to a native binary, a WASM playground,
//! or an in-engine runtime. The MCP server, file encoders, persistence, and
//! daemon live in the `sonarium` shell crate that depends on this one.

pub mod analysis;
pub mod dsl;
pub mod dsp;
pub mod edit;
pub mod render;
pub mod review;
pub mod vary;
