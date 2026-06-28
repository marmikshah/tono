//! Sonarium — a sound-engineering MCP server.
//!
//! An AI agent composes sound by authoring a symbolic synthesis graph
//! (oscillators → envelopes → filters → modulation → mix); Sonarium renders
//! that graph deterministically to audio and feeds back analysis so the agent
//! can iterate by inspection, like a sound designer at a DAW.
//!
//! This crate is the **shell**: the MCP tool surface, file encoders, bank /
//! session persistence, engine emitters, and the daemon. The pure, headless
//! engine — graph DSL, DSP, renderer, analysis, critique, and graph transforms
//! — lives in the [`sonarium_core`] crate and is re-exported here so existing
//! `crate::dsl` / `sonarium::render` paths resolve unchanged.

pub use sonarium_core::{analysis, dsl, dsp, edit, render, review, vary};

pub mod audio;
pub mod bank;
pub mod engines;
pub mod journal;
pub mod midi;
pub mod resources;
pub mod server;
pub mod service;
pub mod session;
