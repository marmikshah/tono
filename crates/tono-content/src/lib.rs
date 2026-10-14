//! Authored content built on [`tono_core`]: the curated sound library and the
//! original background music themes.
//!
//! Everything here is asset-free. [`library`] generates editable, engine-pinned
//! sound designs from 64 recipes in six voicings; [`bgm`] composes twelve
//! themes in three arrangements as seamless four-bar loops with editable
//! scores. The documentation site exports both through the `site_samples`
//! example, and the content hash locks in this crate's tests pin every
//! recipe and theme byte-for-byte.

#![warn(missing_docs)]

pub mod bgm;
pub mod library;
