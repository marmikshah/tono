//! Tono core — the pure, headless audio engine.
//!
//! This crate is the deterministic heart of tono with **no I/O and no
//! transport**. Rendering is a pure function of `(graph, seed, sample_rate)`
//! → byte-identical audio, so a sound is data you can test, diff, and cache:
//!
//! ```
//! use tono_core::dsl::SoundDoc;
//! use tono_core::render;
//!
//! let doc: SoundDoc = serde_json::from_str(r#"{
//!     "name": "blip", "duration": 0.3, "engine": 4,
//!     "root": { "type": "mul", "inputs": [
//!         { "type": "sine", "freq": 880 },
//!         { "type": "env", "a": 0.002, "d": 0.08, "s": 0.0, "r": 0.05 } ] }
//! }"#).unwrap();
//!
//! let a = render::render(&doc);
//! let b = render::render(&doc);
//! assert_eq!(a, b); // byte-identical every run
//! ```
//!
//! # The map
//!
//! Authoring: [`dsl`] (the `SoundDoc` graph + validation) · [`patch`]
//! (templates with named parameters) · [`edit`] (path-addressed edits) ·
//! [`vary`] (deterministic variations).
//!
//! Rendering: [`render`] (the offline bounce) · [`streaming`] (real-time,
//! byte-identical to the bounce) · [`dsp`] (RNG, loudness, limiting) ·
//! [`player`] (buffer playback).
//!
//! Playing live: [`runtime`] (the [`runtime::AudioSource`] seam, `Engine`,
//! `Mixer`, the wait-free split) · [`instrument`] (polyphonic playable
//! voices) · [`drumkit`] · [`adaptive`] (intensity stems, quantized
//! transitions, stingers).
//!
//! Composing: [`song`] (tracks/patterns/arrangement, compiles to a plain
//! `SoundDoc`) · [`catalog`] + [`presets`] (ready-made voices).
//!
//! Feedback: [`analysis`] (stats + spectrogram/waveform images) · [`review`]
//! (grade a sound against its archetype).
//!
//! # Features and the shell
//!
//! The lean build is pure compute (serde only); the heavy deps are optional,
//! behind features (both on by default): `analysis` pulls in rustfft + image for
//! [`analysis`]/[`review`], and `sampler` pulls in rustysynth for the SoundFont
//! sampler instrument. So the same core compiles to a native binary, a WASM
//! playground, or a lean in-engine runtime. The `tono render` CLI, audio-file
//! encoders, and MIDI export live in the `tono` shell crate that depends on this one.
//!
//! Longer-form guides: the [cookbook] (the node vocabulary + recipes) and the
//! [architecture guide] (how the pieces compose, bottom-up).
//!
//! [cookbook]: https://github.com/marmikshah/tono/blob/master/docs/cookbook.md
//! [architecture guide]: https://marmikshah.github.io/tono/architecture.html

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

/// Renamed to [`player`] — `stream` (the buffer-backed audition `Player`) sat
/// one suffix away from [`streaming`] (the per-sample block renderer), and the
/// pair was a reliable source of confusion. Deleted at 2.0.
#[deprecated(since = "1.6.0", note = "renamed to `player`")]
pub use player as stream;

/// Moved to [`instrument`] — this module was named for a type it doesn't
/// contain (`EnvGen`'s only consumer is the instrument; the crate's `Voice`
/// lives in [`catalog`]). This shim keeps `tono_core::voice::EnvGen` valid
/// until 2.0.
pub mod voice {
    pub use crate::instrument::EnvGen;
}

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
