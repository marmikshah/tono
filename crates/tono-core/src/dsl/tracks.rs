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
    /// Sidechain ducking: this track's level dips whenever the `source`
    /// track's signal is loud — the classic kick→bass pump, at mixer level.
    /// The source track renders exactly as it does today; only this (the
    /// follower) track is gain-reduced. None ⇒ the render is byte-identical
    /// to a document without this field. A sidechained mix streams natively
    /// (the duck envelope advances per sample, so a schema-v2 `tracks` root
    /// — sidechains, buses, and all — streams byte-identically to the
    /// offline bounce).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sidechain: Option<Sidechain>,
    /// The mix bus this track's main output routes to (e.g. `"drums"`,
    /// `"reverb"`). None routes directly to the master bus.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bus: Option<String>,
    /// Post-fader sends: copies of this track's positioned stereo signal
    /// (post fader/pan/duck), each scaled by its amount, into mix buses.
    /// Empty ⇒ the render is byte-identical to a document without this field.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sends: Vec<Send>,
}

/// A mix bus in a [`Node::Tracks`] root: a named submix with its own insert
/// chain, returned onto the master bus. Tracks route to it with their `bus`
/// field and feed it with `sends`; the rendered mix without it is
/// byte-identical, so buses are purely additive.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct Bus {
    /// Stable bus id — a short slug like `"drums"` or `"reverb"`, unique
    /// within the document (and never colliding with a track id).
    pub id: String,
    /// The bus return fader, 0..2 (1 = unity).
    #[serde(default = "default_gain")]
    pub gain: f32,
    /// Insert chain on the bus (stereo processors, applied like the master
    /// chain — a reverb gets the decorrelated-tails treatment).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub effects: Vec<Node>,
}

/// A post-fader send into a mix bus.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
pub struct Send {
    /// The target bus's id.
    pub bus: String,
    /// Send level, 0..1 (0 = silent, 1 = the full post-fader signal).
    #[serde(default = "default_send_amount")]
    pub amount: f32,
}

fn default_send_amount() -> f32 {
    0.5
}

/// A tracks-level sidechain link: the follower's post-fader signal is
/// multiplied by a gain envelope driven by the `source` track's signal, with
/// the same attack/release follower the `duck` node uses (so the pump
/// character matches). A source must be a plain track — follower-of-follower
/// chains are rejected by validation (duck directly to the source's source).
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
pub struct Sidechain {
    /// The id of the track whose signal drives the ducking (e.g. `"kick"`).
    pub source: String,
    /// Duck depth, 0..1 (1 = fully silent at the source's peak).
    #[serde(default = "default_sidechain_amount")]
    pub amount: f32,
    /// Gain-reduction attack in seconds.
    #[serde(default = "default_sidechain_attack")]
    pub attack: f32,
    /// Recovery time in seconds (the "pump" length).
    #[serde(default = "default_sidechain_release")]
    pub release: f32,
}

// The defaults mirror the `duck` node's, so moving a pump from inside a node
// tree to the mixer keeps the same feel.
fn default_sidechain_amount() -> f32 {
    0.8
}
fn default_sidechain_attack() -> f32 {
    0.005
}
fn default_sidechain_release() -> f32 {
    0.25
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

/// How an automation lane interpolates between its breakpoints.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum AutoCurve {
    /// Straight-line segments (default).
    #[default]
    Linear,
    /// Hold the previous breakpoint's value until the next one lands (a
    /// stepped ride — fader moves without ramps).
    Step,
    /// Exponential approach per segment: `v0 · (v1/v0)^u` while both
    /// endpoints are positive (natural-feeling swells and fades); any other
    /// segment falls back to linear — deterministic, and documented.
    Exp,
}

/// One breakpoint in an automation lane: value `v` at song time `t` seconds.
/// Between breakpoints the value follows the lane's `curve`; before the
/// first / after the last it holds flat.
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
    /// The interpolation between breakpoints (default linear).
    #[serde(default)]
    pub curve: AutoCurve,
    /// Breakpoints over song time.
    pub points: Vec<AutoPoint>,
}
