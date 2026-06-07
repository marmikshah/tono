//! Sonarium — a sound-engineering MCP server.
//!
//! An AI agent composes sound by authoring a symbolic synthesis graph
//! (oscillators → envelopes → filters → modulation → mix); Sonarium renders
//! that graph deterministically to audio and feeds back analysis so the agent
//! can iterate by inspection, like a sound designer at a DAW.
//!
//! The crate is a library plus a thin binary (`src/main.rs`). Modules are
//! added bottom-up: the DSL data model first, then DSP, rendering, state,
//! and finally the MCP tool surface.

pub mod bank;
pub mod dsl;
pub mod dsp;
