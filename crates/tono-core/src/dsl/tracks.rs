//! Mixer-track types: one channel of a [`Node::Tracks`] root plus its
//! automation lanes.

use super::{Node, default_gain};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// One mixer channel in a [`Node::Tracks`] root.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct Track {
    /// Stable layer id — a short slug like `"kick"` or `"tail"`, unique within
    /// the document. This is how edits address the track by id, so it never
    /// shifts when sibling layers are added or
    /// removed (unlike an array index). Omitted ids are backfilled
    /// deterministically (`layer_<position>`) on the next build.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    /// The track's signal graph (usually a `seq` or a `chain`).
    pub node: Node,
    /// Stereo position, −1 (hard left) .. 1 (hard right). Equal-power law.
    #[serde(default)]
    pub pan: f32,
    /// Channel fader, 0..2 (1 = unity).
    #[serde(default = "default_gain")]
    pub gain: f32,
    /// Start offset in seconds: the rendered layer is shifted this far right
    /// on the bus (the transient + body + tail recipe). The render keeps its
    /// full length and the shifted tail is truncated at the document edge.
    #[serde(default)]
    pub at: f32,
    /// Muted layers stay in the document but are left off the bus. This is
    /// rendered state, not a monitoring convenience — exports ship without
    /// muted layers.
    #[serde(default)]
    pub mute: bool,
    /// Song-time automation lanes for this track's `gain` / `pan` (volume rides,
    /// pan moves across sections). Empty ⇒ the static `gain`/`pan` apply and the
    /// render is byte-identical to a document without this field. A lane's value
    /// overrides the static one over time; per-node modulators still cover the
    /// node level (this is the track/song level).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub automation: Vec<AutoLane>,
}

/// What a track automation lane controls.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum AutoTarget {
    /// The track's channel fader (0..2).
    Gain,
    /// The track's stereo position (−1..1).
    Pan,
}

/// One breakpoint in an automation lane: value `v` at song time `t` seconds.
/// Between breakpoints the value is linearly interpolated; before the first /
/// after the last it holds flat.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct AutoPoint {
    /// Song time in seconds.
    pub t: f32,
    /// Target value at this time.
    pub v: f32,
}

/// A track automation lane: a `target` driven by a list of breakpoints.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct AutoLane {
    /// What this lane controls.
    pub target: AutoTarget,
    /// Breakpoints over song time.
    pub points: Vec<AutoPoint>,
}
