//! Sound banks: named groups of sounds for cohesive, engine-wireable packs.
//!
//! A [`Bank`] is a first-class, addressable set. Export a bank and you get
//! every member audio file plus a single `sounds.json` manifest a game engine
//! can read to wire the whole pack — with categories and round-robin groups —
//! without hand-listing files.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// A named group of sounds (a "sound pack").
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct Bank {
    /// Stable id (slug of the name), also the manifest/asset key.
    pub id: String,
    /// Human-readable label.
    pub name: String,
    /// The sounds in this bank, with engine-facing metadata.
    #[serde(default)]
    pub members: Vec<BankMember>,
}

/// One sound's membership in a bank, with metadata the engine reads at export.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct BankMember {
    /// Id of the sound in the session store.
    pub sound_id: String,
    /// Logical category (e.g. `"ui"`, `"weapon"`, `"footstep"`). Used to lay out
    /// export subdirectories and to group the manifest.
    #[serde(default)]
    pub category: Option<String>,
    /// Round-robin group: members sharing a group are interchangeable takes the
    /// engine can cycle so repeats never sound identical.
    #[serde(default)]
    pub rr_group: Option<String>,
}

/// One entry in an exported pack's `sounds.json` manifest.
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct ManifestEntry {
    pub id: String,
    pub name: String,
    /// Path to the audio file relative to the export directory.
    pub file: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub category: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rr_group: Option<String>,
    /// True when the file is a seamless loop (WAV exports also carry a `smpl`
    /// chunk spanning the whole file).
    #[serde(rename = "loop")]
    pub looped: bool,
    pub duration_ms: u32,
    pub sample_rate: u32,
    pub channels: u16,
    pub lufs: f32,
    pub peak_dbfs: f32,
    pub true_peak_dbfs: f32,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manifest_entry_wire_shape() {
        // `loop` (a Rust keyword) is the manifest field name engines read;
        // empty optionals stay out of the JSON entirely.
        let e = ManifestEntry {
            id: "laser".into(),
            name: "Laser".into(),
            file: "laser.wav".into(),
            category: None,
            rr_group: None,
            looped: true,
            duration_ms: 220,
            sample_rate: 44_100,
            channels: 1,
            lufs: -16.0,
            peak_dbfs: -1.0,
            true_peak_dbfs: -0.8,
        };
        let v = serde_json::to_value(&e).unwrap();
        assert_eq!(v["loop"], true);
        assert!(v.get("looped").is_none());
        assert!(v.get("category").is_none());
    }

    #[test]
    fn bank_members_default_to_empty() {
        let b: Bank = serde_json::from_str(r#"{ "id": "ui", "name": "UI" }"#).unwrap();
        assert!(b.members.is_empty());
    }
}
