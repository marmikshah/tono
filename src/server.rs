//! The MCP server: tool surface over the synthesis pipeline.
//!
//! Tools form the author → render → analyze → refine loop:
//! `author_sound` / `refine_sound` produce audio + analysis (stats and inline
//! spectrogram/waveform images), `get_sound` / `list_sounds` / `analyze`
//! inspect, and `export` writes a final game-ready file. Every mutating call is
//! recorded to the session journal so `save_session` / `replay_session` can
//! reproduce an entire session deterministically.

use std::sync::Arc;

use base64::Engine as _;
use rmcp::{
    ErrorData, Json, RoleServer, ServerHandler,
    handler::server::{router::tool::ToolRouter, wrapper::Parameters},
    model::{
        Annotated, CallToolResult, Content, Implementation, ListResourcesResult,
        PaginatedRequestParams, RawResource, ReadResourceRequestParams, ReadResourceResult,
        ResourceContents, ServerCapabilities, ServerInfo,
    },
    service::RequestContext,
    tool, tool_handler, tool_router,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::analysis::{self, Analysis};
use crate::audio;
use crate::bank::{Bank, BankMember, ManifestEntry};
use crate::dsl::{Adsr, Node, NoiseColor, Normalize, Playback, SoundDoc, Stereo, Track, Value};
use crate::dsp::dbfs;
use crate::edit::{self, EditOp, NodeInfo};
use crate::engines::{self, EngineTarget};
use crate::journal::{self, Journal};
use crate::render;
use crate::resources;
use crate::review::{self, Archetype, Review};
use crate::session::{Record, Store, now_secs, slugify};
use crate::vary;

const INSTRUCTIONS: &str = "Sonarium is a sound-engineering MCP server: you compose audio from instruments and effects by authoring a symbolic synthesis graph, and the server renders it deterministically, returning analysis (peak/true-peak/RMS, LUFS, spectral centroid, transients) plus two images — a spectrogram and a waveform — so you iterate by inspection, like a sound designer at a DAW.\n\
Workflow: author_sound with a graph (the sound's FIRST layer) → read the stats, view the images → add_layer to stack each next instrument/component under a stable layer id (one layer per thing you'd fade, pan, time-shift, or analyze separately: transient/body/tail for SFX, one instrument each for music; every render reports each layer's pre-master contribution) → refine with set_param / edit_sound (surgical; with layer set, paths are layer-relative like env.a or notes[3].pitch — call describe_sound first for the map), set_layer for mixer moves (gain/pan/at/mute), or refine_sound (whole-graph replace) → export (wav/flac/ogg) when it matches. undo_sound / redo_sound step a 100-deep per-sound history; sounds persist across restarts under stable slug ids.\n\
Graph vocabulary: root is one node; every node is a mono signal. Sources: square{freq,duty} (duty modulatable ⇒ PWM), triangle{freq}, sawtooth{freq}, sine{freq}, noise{color: white|pink|brown}, fm{freq,ratio,index} (bells / metallic), super{wave,freq,voices,detune_cents} (supersaw), and seq{bpm,steps_per_beat,wave,duty,env,notes} for melodies/basslines/drums — each note has its own pitch (a number, a NOTE NAME like \"C4\"/\"F#3\"/\"midi:60\", or a slide), a length in grid steps, and the shared per-note ADSR; gaps are rests; notes may overlap. Seq waves are square/triangle/sawtooth/sine/noise plus a core INSTRUMENT list: piano (acoustic — velocity brightness, bass rings/treble dies), epiano (Rhodes tine), organ (drawbars, sustains while held), strings (slow-swell ensemble — write notes slightly early), bass (filtered + sub), kit (drums on the General MIDI map — pitch picks the drum: midi:36 kick, 38 snare, 42/46 hats, 41-50 toms, 49 crash, 51 ride), fm (tunable mallets/bells via fm_ratio/fm_index/fm_strike), pluck (Karplus-Strong string via pluck_decay), cowbell (phonk lead), and sampler — REAL recorded instruments from any SoundFont: set sf2 to a .sf2 path and sf2_preset to the General MIDI program (0 piano, 32 bass, 48 strings; sf2_bank 128 = GM drum map). Every seq also takes swing (shuffle) and humanize (deterministic timing/velocity jitter), and the duck processor sidechain-pumps any chain to a trigger. For full productions use a top-level tracks root — the mixing console: tracks:[{id, node, pan(-1..1), gain, at, mute}] places every layer on the stereo stage by stable id (sampler tracks keep native stereo; `at` time-shifts a layer; each layer has its own deterministic RNG stream) and master:[processors] is the stereo bus chain with decorrelated reverb tails (address them at root.master[i]). mix still layers mono inside one track — but prefer real layers for anything you'd balance separately. Envelope: env{a,d,s,r,punch}. Combinators: mix (sum), mul (source × env), chain (source → processors). Processors: lowpass/highpass/bandpass/notch{cutoff,q}, peak{cutoff,q,gain_db}, lowshelf/highshelf{cutoff,gain_db}, gain, drive{amount,shape}, ringmod, chorus, flanger, phaser, compress, bitcrush, downsample, delay, reverb. Any numeric param may be a modulator: {\"slide\":{from,to,secs,curve}}, {\"lfo\":{shape,rate,depth,center}}, {\"arp\":{steps,rate}}, {\"env\":{a,d,s,r,from,to}} (the key to filter/pitch envelopes).\n\
Output shaping: add top-level stereo {\"mode\":\"wide\"|\"haas\"} for BGM/ambience width, playback {\"mode\":\"loop\",crossfade_secs} for a seamless loop (or call make_loop on an existing sound — the exported WAV carries a smpl loop chunk engines read), and normalize {target_lufs,ceiling_dbtp} for level-matched, click-safe renders. export also takes target_lufs.\n\
Variations on sounds you made: mutate_sound nudges parameters; generate_variants makes N level-matched round-robin takes of a sound; humanize applies one coherent pitch shift + level trim per take (foley repeats); morph_sounds interpolates two same-shaped designs (charge tiers). compare_sounds reports metric deltas + a similarity score to converge on a reference.\n\
Packs: create_bank / add_to_bank{category?,rr_group?} / list_banks, then export_bank or export_all write every member (wav/flac/ogg) plus a sounds.json manifest; pass engine:\"godot\"|\"unity\"|\"bevy\" to also emit engine integration files.\n\
Sessions are reproducible: every mutating call is journaled to session.jsonl in the working directory; save_session snapshots it and replay_session re-applies a saved journal — same calls, same seeds, byte-identical audio. Read the resources (DSL JSON Schema + cookbook) for the full vocabulary and worked examples.";

/// The Sonarium MCP server.
///
/// `Clone` shares all state (store, journal, replay flag): in HTTP mode one
/// handler is built and cloned per session, so concurrent sessions append to
/// the journal through ONE mutex instead of racing on the file.
#[derive(Clone)]
pub struct Sonarium {
    store: Arc<Store>,
    journal: Arc<Journal>,
    /// True while `replay_session` runs: per-tool journaling is suppressed and
    /// the replayed steps are copied into the journal verbatim instead.
    replaying: Arc<std::sync::atomic::AtomicBool>,
    tool_router: ToolRouter<Self>,
}

// ---------- Tool request / response shapes ----------

/// Author a new sound from a graph.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct AuthorReq {
    /// The synthesis graph to render.
    pub graph: SoundDoc,
    /// Optional display name (overrides the graph's `name`).
    #[serde(default)]
    pub name: Option<String>,
}

/// Scaffold a blank 4-layer SFX document (sub / body / top / transient).
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct ScaffoldReq {
    /// Fundamental of the body layer in Hz (the sub sits an octave below).
    /// Defaults to 220.
    #[serde(default)]
    pub base_freq: Option<f32>,
    /// Seed for the noise layers' deterministic streams. Defaults to 0.
    #[serde(default)]
    pub seed: Option<u64>,
    /// Display name for the new sound. Defaults to `"layered_sfx"`.
    #[serde(default)]
    pub name: Option<String>,
}

/// Replace an existing sound's graph and re-render.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct RefineReq {
    /// Id of the sound to refine.
    pub id: String,
    /// The new graph.
    pub graph: SoundDoc,
}

/// Reference an existing sound by id.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct IdReq {
    /// Id of the sound.
    pub id: String,
}

/// Output container for export targets.
#[derive(Debug, Clone, Copy, Default, Deserialize, JsonSchema, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum ExportFormat {
    /// WAV (PCM; carries the `smpl` loop chunk for looped sounds).
    #[default]
    Wav,
    /// FLAC (lossless compression).
    Flac,
    /// OGG Vorbis (lossy VBR — the usual shipping format for BGM / ambience).
    Ogg,
}

impl ExportFormat {
    fn ext(self) -> &'static str {
        match self {
            ExportFormat::Wav => "wav",
            ExportFormat::Flac => "flac",
            ExportFormat::Ogg => "ogg",
        }
    }
}

/// Export a sound to a game-ready file.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct ExportReq {
    /// Id of the sound to export.
    pub id: String,
    /// Output format.
    #[serde(default)]
    pub format: ExportFormat,
    /// Bit depth (8 or 16). Defaults to 16.
    #[serde(default)]
    pub bit_depth: Option<u16>,
    /// Override the sample rate (re-renders at this rate).
    #[serde(default)]
    pub sample_rate: Option<u32>,
    /// Destination path. Defaults to the working directory.
    #[serde(default)]
    pub dest: Option<String>,
    /// Optional integrated-loudness target (LUFS) for the exported file, with a
    /// −1 dBTP true-peak ceiling. Use it to write a level-matched, click-safe
    /// game asset without editing the stored graph.
    #[serde(default)]
    pub target_lufs: Option<f32>,
    /// OGG Vorbis VBR quality in [0, 1] (default 0.5 ≈ transparent for game
    /// audio). Ignored for WAV / FLAC.
    #[serde(default)]
    pub quality: Option<f32>,
}

/// Full record returned by `get_sound`.
#[derive(Debug, Serialize, JsonSchema)]
pub struct GetSoundResp {
    pub id: String,
    pub name: String,
    pub graph: SoundDoc,
    pub wav_path: String,
    pub analysis: Analysis,
}

/// One row in `list_sounds`.
#[derive(Debug, Serialize, JsonSchema)]
pub struct SoundSummary {
    pub id: String,
    pub name: String,
    pub duration: f32,
}

/// Inventory returned by `list_sounds`.
#[derive(Debug, Serialize, JsonSchema)]
pub struct ListResp {
    pub sounds: Vec<SoundSummary>,
}

/// Result of `export`.
#[derive(Debug, Serialize, JsonSchema)]
pub struct ExportResp {
    pub path: String,
}

fn default_amount() -> f32 {
    0.2
}

/// Perturb an existing sound's graph into a new variant.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct MutateReq {
    /// Id of the sound to mutate.
    pub id: String,
    /// How far to nudge parameters, 0..1 (default 0.2).
    #[serde(default = "default_amount")]
    pub amount: f32,
    /// Seed for reproducibility. Omit for a fresh variant (the resolved seed is
    /// what lands in the session journal, so replays stay deterministic).
    #[serde(default)]
    pub seed: Option<u64>,
}

fn default_variant_amount() -> f32 {
    0.15
}
fn default_variant_lufs() -> f32 {
    -16.0
}

/// Generate a round-robin set of variations of an existing sound.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct VariantsReq {
    /// Existing sound id to base the variants on.
    pub id: String,
    /// How many variants to produce (1..=32).
    pub count: u32,
    /// Per-variant perturbation amount, 0..1 (default 0.15).
    #[serde(default = "default_variant_amount")]
    pub amount: f32,
    /// Base seed for reproducibility.
    #[serde(default)]
    pub seed: Option<u64>,
    /// Integrated-loudness target (LUFS) every variant is matched to, so
    /// round-robin takes play back at the same perceived level. Default −16.
    #[serde(default = "default_variant_lufs")]
    pub target_lufs: f32,
}

/// One row in a `generate_variants` / `morph_sounds` / `humanize` result.
#[derive(Debug, Serialize, JsonSchema)]
pub struct VariantSummary {
    pub id: String,
    pub name: String,
    pub wav_path: String,
    pub loudness_lufs: f32,
    pub peak_dbfs: f32,
    pub spectral_centroid_hz: f32,
}

/// Result of a multi-sound generator.
#[derive(Debug, Serialize, JsonSchema)]
pub struct VariantsResp {
    pub count: u32,
    pub variants: Vec<VariantSummary>,
}

/// The addressing map for a sound's graph.
#[derive(Debug, Serialize, JsonSchema)]
pub struct DescribeResp {
    pub id: String,
    pub name: String,
    pub duration: f32,
    /// Node rows of a plain (non-mixer) document, absolute paths from `root`.
    pub nodes: Vec<NodeInfo>,
    /// Mixer layers keyed by stable id; node paths inside are layer-relative
    /// (pass them with `layer` to set_param / edit_sound). Empty for plain docs.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub layers: Vec<edit::LayerInfo>,
    /// Master-chain processors (absolute paths, no `layer` arg).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub master: Vec<NodeInfo>,
}

/// Set a single parameter (or node) by path.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct SetParamReq {
    /// Id of the sound to edit.
    pub id: String,
    /// Address a mixer layer by its stable id; `path` is then relative to the
    /// layer's node (e.g. `env.a`, `notes[3].pitch`, `stages[1].cutoff`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub layer: Option<String>,
    /// Path to the target, e.g. `root.inputs[0].freq` — or, with `layer`,
    /// layer-relative like `inputs[0].freq`.
    pub path: String,
    /// New value: a number, a modulator object
    /// (`{"slide":{"from":880,"to":180,"secs":0.18}}`), or a whole node.
    pub value: serde_json::Value,
}

/// Apply a batch of edit ops to a sound.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct EditReq {
    /// Id of the sound to edit.
    pub id: String,
    /// Address a mixer layer by its stable id; every op's path is then
    /// relative to that layer's node.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub layer: Option<String>,
    /// Ordered edit ops applied in one re-render.
    pub ops: Vec<EditOp>,
}

/// Add an instrument layer to a sound's mixer.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct AddLayerReq {
    /// Id of the sound to extend.
    pub id: String,
    /// The new layer's stable id — a short slug like `kick`, `body`, `tail`.
    /// Duplicates are rejected (every layer stays unambiguously addressable).
    pub layer: String,
    /// The layer's signal graph: any source/combinator node (seq, chain, mix…).
    pub node: Node,
    /// Fader 0..2 (default 1).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gain: Option<f32>,
    /// Stereo position −1..1 (default 0, center).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pan: Option<f32>,
    /// Start offset in seconds (default 0) — place a tail layer late, a
    /// pre-transient click early.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub at: Option<f32>,
}

/// Adjust a layer's mixer fields without touching its graph.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct SetLayerReq {
    /// Id of the sound.
    pub id: String,
    /// The layer to adjust (see describe_sound for the list).
    pub layer: String,
    /// New fader value 0..2.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gain: Option<f32>,
    /// New stereo position −1..1.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pan: Option<f32>,
    /// New start offset in seconds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub at: Option<f32>,
    /// Mute / unmute. Mute is rendered state: exports ship without muted layers.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mute: Option<bool>,
}

/// Structural layer operations.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum LayerOp {
    /// Delete the layer (a mixer keeps at least one).
    Remove,
    /// Copy the layer under `new_id` (its noise re-realizes deterministically
    /// from the new id — a built-in variation).
    Duplicate,
}

/// Remove or duplicate a layer.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct LayerOpsReq {
    /// Id of the sound.
    pub id: String,
    /// The operation.
    pub op: LayerOp,
    /// The layer to operate on.
    pub layer: String,
    /// New layer id for `duplicate`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub new_id: Option<String>,
}

/// Operation for the unified `layer` tool.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum LayerKind {
    /// Stack a new instrument layer (requires `node`).
    Add,
    /// Mixer move on an existing layer (gain/pan/at/mute), graph untouched.
    Set,
    /// Remove a layer.
    Remove,
    /// Duplicate a layer (re-grains noise deterministically; needs `new_id`).
    Duplicate,
}

/// Add, adjust, or restructure a mixer layer.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct LayerReq {
    /// Id of the sound.
    pub id: String,
    /// What to do.
    pub op: LayerKind,
    /// The layer id to operate on (the new layer's id for `add`).
    pub layer: String,
    /// The layer's graph — required for `add`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub node: Option<Node>,
    /// Channel gain (0..2) — `add` / `set`.
    #[serde(default)]
    pub gain: Option<f32>,
    /// Stereo pan (−1..1) — `add` / `set`.
    #[serde(default)]
    pub pan: Option<f32>,
    /// Start offset in seconds — `add` / `set`.
    #[serde(default)]
    pub at: Option<f32>,
    /// Mute state — `set`.
    #[serde(default)]
    pub mute: Option<bool>,
    /// New layer id — `duplicate`.
    #[serde(default)]
    pub new_id: Option<String>,
}

fn default_loop_crossfade() -> f32 {
    0.1
}
fn default_morph_steps() -> u32 {
    3
}
fn default_humanize_count() -> u32 {
    4
}
fn default_humanize_cents() -> f32 {
    30.0
}
fn default_humanize_db() -> f32 {
    1.5
}

/// Create in-betweens of two same-shaped sounds.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct MorphReq {
    /// Id of the first sound (t = 0).
    pub a: String,
    /// Id of the second sound (t = 1). Must share the first's graph shape.
    pub b: String,
    /// How many in-between sounds to create (1..=10, default 3).
    #[serde(default = "default_morph_steps")]
    pub steps: u32,
}

/// Musically-aware round-robin takes.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct HumanizeReq {
    /// Id of the sound to humanize.
    pub id: String,
    /// How many takes (1..=16, default 4).
    #[serde(default = "default_humanize_count")]
    pub count: u32,
    /// Max pitch deviation in cents (default 30 — a real performer's spread).
    #[serde(default = "default_humanize_cents")]
    pub pitch_cents: f32,
    /// Max level deviation in dB (default 1.5).
    #[serde(default = "default_humanize_db")]
    pub gain_db: f32,
    /// Base seed for reproducibility.
    #[serde(default)]
    pub seed: Option<u64>,
}

/// Turn a sound into a seamless loop.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct MakeLoopReq {
    /// Id of the sound to loop.
    pub id: String,
    /// Equal-power crossfade length in seconds (default 0.1).
    #[serde(default = "default_loop_crossfade")]
    pub crossfade_secs: f32,
    /// Loop start in seconds (default 0).
    #[serde(default)]
    pub start_secs: f32,
    /// Loop end in seconds (default: end of the rendered buffer).
    #[serde(default)]
    pub end_secs: Option<f32>,
}

/// Compare two sounds.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct CompareReq {
    /// Id of the first (reference) sound.
    pub a: String,
    /// Id of the second sound, compared against `a`.
    pub b: String,
}

/// Review a sound against archetype targets + the ship checklist.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct ReviewReq {
    /// Id of the sound to review.
    pub id: String,
    /// Archetype to grade against (`laser` / `coin` / `jump` / `impact` /
    /// `ui` / `ambience` / `bgm`). Omit to run only the universal checks
    /// (clipping, true-peak, head/tail silence, loop seam).
    #[serde(default)]
    pub archetype: Option<Archetype>,
}

/// Metric deltas (`b − a`) plus an overall similarity score.
#[derive(Debug, Serialize, JsonSchema)]
pub struct CompareResp {
    pub a: String,
    pub b: String,
    /// 0..1, 1 = effectively identical on the measured axes.
    pub similarity: f32,
    pub centroid_delta_hz: f32,
    pub lufs_delta: f32,
    pub peak_delta_db: f32,
    pub crest_delta_db: f32,
    pub attack_delta_ms: f32,
    pub decay_delta_ms: f32,
    pub duration_delta_ms: f32,
}

/// Undo/redo stack depths for a sound.
#[derive(Debug, Serialize, JsonSchema)]
pub struct HistoryResp {
    pub id: String,
    /// Number of revisions `undo_sound` can step back through.
    pub undo_depth: usize,
    /// Number of states `redo_sound` can re-apply.
    pub redo_depth: usize,
}

/// Operation for the `history` tool.
#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum HistoryOp {
    /// Report undo/redo depths (default).
    #[default]
    Status,
    /// Revert to the previous graph; the undone state moves to the redo stack.
    Undo,
    /// Re-apply the most recently undone edit.
    Redo,
}

/// Step through or inspect a sound's revision history.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct HistoryReq {
    /// Id of the sound.
    pub id: String,
    /// What to do (default `status`).
    #[serde(default)]
    pub op: HistoryOp,
}

/// Create a sound bank.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct CreateBankReq {
    /// Human-readable bank name (its id is the slug of this).
    pub name: String,
}

/// Add/update a sound's membership in a bank.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct AddToBankReq {
    /// Id of the bank.
    pub bank_id: String,
    /// Id of the sound to add.
    pub sound_id: String,
    /// Optional logical category (`ui`, `weapon`, `footstep`, ...).
    #[serde(default)]
    pub category: Option<String>,
    /// Optional round-robin group: members sharing a group are interchangeable takes.
    #[serde(default)]
    pub rr_group: Option<String>,
}

/// Inventory of banks.
#[derive(Debug, Serialize, JsonSchema)]
pub struct BanksResp {
    pub banks: Vec<Bank>,
}

/// Unified result of the `bank` tool: a single bank (`create` / `add`) or the
/// full inventory (`list`).
#[derive(Debug, Serialize, JsonSchema)]
pub struct BankResp {
    /// The affected bank, for `create` / `add`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bank: Option<Bank>,
    /// Every bank, for `list`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub banks: Option<Vec<Bank>>,
}

/// Operation for the unified `bank` tool.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum BankKind {
    /// Create a named bank (requires `name`).
    Create,
    /// Add/update a sound's membership (requires `bank_id`, `sound_id`).
    Add,
    /// List all banks and their members.
    List,
}

/// Create a bank, add a sound to one, or list them.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct BankReq {
    /// What to do.
    pub op: BankKind,
    /// Bank name — `create`.
    #[serde(default)]
    pub name: Option<String>,
    /// Bank id — `add`.
    #[serde(default)]
    pub bank_id: Option<String>,
    /// Sound id to add — `add`.
    #[serde(default)]
    pub sound_id: Option<String>,
    /// Logical category (`ui`, `weapon`, ...) — `add`.
    #[serde(default)]
    pub category: Option<String>,
    /// Round-robin group (interchangeable takes) — `add`.
    #[serde(default)]
    pub rr_group: Option<String>,
}

/// Export a bank to a directory + manifest.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct ExportBankReq {
    /// Id of the bank to export.
    pub bank_id: String,
    /// Destination directory (created if missing).
    pub dest: String,
    /// Lay sounds into per-category subfolders.
    #[serde(default)]
    pub by_category: bool,
    /// Optional integrated-loudness target (LUFS) applied to every member.
    #[serde(default)]
    pub target_lufs: Option<f32>,
    /// Container format for every member (wav / flac / ogg). Defaults to WAV.
    #[serde(default)]
    pub format: ExportFormat,
    /// OGG Vorbis VBR quality in [0, 1] (default 0.5). Ignored for WAV / FLAC.
    #[serde(default)]
    pub quality: Option<f32>,
    /// Also emit engine integration files: godot (.import sidecars with loop
    /// mode), unity (.meta sidecars with stable GUIDs), or bevy (a generated
    /// sonarium_sounds.rs asset-key module).
    #[serde(default)]
    pub engine: Option<EngineTarget>,
}

/// Export the entire library to a directory + manifest.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct ExportAllReq {
    /// Destination directory (created if missing).
    pub dest: String,
    /// Optional integrated-loudness target (LUFS) applied to every sound.
    #[serde(default)]
    pub target_lufs: Option<f32>,
    /// Container format for every sound (wav / flac / ogg). Defaults to WAV.
    #[serde(default)]
    pub format: ExportFormat,
    /// OGG Vorbis VBR quality in [0, 1] (default 0.5). Ignored for WAV / FLAC.
    #[serde(default)]
    pub quality: Option<f32>,
    /// Also emit engine integration files (godot / unity / bevy).
    #[serde(default)]
    pub engine: Option<EngineTarget>,
}

/// Export a pack: one bank, or the whole library when `bank_id` is omitted.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct ExportPackReq {
    /// Bank id to export. Omit to export the entire library.
    #[serde(default)]
    pub bank_id: Option<String>,
    /// Destination directory (created if missing).
    pub dest: String,
    /// Lay sounds into per-category subfolders (bank export only).
    #[serde(default)]
    pub by_category: bool,
    /// Optional integrated-loudness target (LUFS) applied to every member.
    #[serde(default)]
    pub target_lufs: Option<f32>,
    /// Container format for every member (wav / flac / ogg). Defaults to WAV.
    #[serde(default)]
    pub format: ExportFormat,
    /// OGG Vorbis VBR quality in [0, 1] (default 0.5). Ignored for WAV / FLAC.
    #[serde(default)]
    pub quality: Option<f32>,
    /// Also emit engine integration files: godot / unity / bevy.
    #[serde(default)]
    pub engine: Option<EngineTarget>,
}

/// Options for a pack export (shared by `export_bank` / `export_all`).
struct PackOptions {
    by_category: bool,
    target_lufs: Option<f32>,
    format: ExportFormat,
    quality: f32,
    engine: Option<EngineTarget>,
}

/// Result of a pack export.
#[derive(Debug, Serialize, JsonSchema)]
pub struct PackResp {
    /// Path to the written `sounds.json` manifest.
    pub manifest_path: String,
    /// Number of sounds exported.
    pub count: u32,
    pub entries: Vec<ManifestEntry>,
    /// Engine integration files written (when `engine` was requested).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub engine_files: Vec<String>,
}

/// Snapshot the session journal to a file.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct SaveSessionReq {
    /// Destination path for the session file. Defaults to
    /// `session_save.jsonl` in the working directory.
    #[serde(default)]
    pub dest: Option<String>,
}

/// Result of `save_session`.
#[derive(Debug, Serialize, JsonSchema)]
pub struct SaveSessionResp {
    pub path: String,
    /// Number of recorded tool calls in the file.
    pub steps: u32,
}

/// Replay a saved session file.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct ReplaySessionReq {
    /// Path to a session file (as written by `save_session`, or a copied
    /// `session.jsonl`).
    pub path: String,
}

/// Result of `replay_session`.
#[derive(Debug, Serialize, JsonSchema)]
pub struct ReplaySessionResp {
    /// Number of tool calls re-applied.
    pub applied: u32,
}

#[tool_router(router = tool_router)]
impl Sonarium {
    /// Construct the server over a session store (the journal lives in the
    /// store's working directory).
    pub fn new(store: Arc<Store>) -> Self {
        let journal = Arc::new(Journal::new(store.dir()));
        Self {
            store,
            journal,
            replaying: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            tool_router: Self::tool_router(),
        }
    }

    /// Parse → render → analyze a graph in one call. The primary tool.
    #[tool(
        name = "author_sound",
        description = "Render a synthesis graph to a WAV and return analysis (stats + spectrogram and waveform images). The primary authoring tool."
    )]
    pub async fn author_sound(
        &self,
        params: Parameters<AuthorReq>,
    ) -> Result<CallToolResult, String> {
        let mut req = params.0;
        // New documents follow the current render semantics. The resolved
        // version is stamped into the journaled args (like mutate_sound's
        // seed) so a future sonarium replays this step byte-faithfully.
        // During replay the stamp must NOT run: a journaled step missing
        // `version` predates stamping and was recorded under v1 semantics.
        if !self.replaying.load(std::sync::atomic::Ordering::SeqCst) {
            req.graph.version.get_or_insert(crate::dsl::SCHEMA_VERSION);
            // New documents render under the current DSP kernels. A journaled
            // step missing `engine` predates this stamping and was recorded
            // under engine 0 — so, like `version`, the stamp must NOT run
            // during replay, or an old session's bytes would shift.
            req.graph.engine.get_or_insert(crate::dsl::ENGINE_VERSION);
        }
        let args = serde_json::to_value(&req).map_err(|e| e.to_string())?;
        let AuthorReq { mut graph, name } = req;
        if let Some(n) = name {
            graph.name = n;
        }
        let id = self.store.unique_id(&graph.name);
        let rec = self.build(id, graph, now_secs())?;
        self.jlog("author_sound", args);
        Ok(sound_result(&rec, false))
    }

    /// Scaffold a blank, band-disciplined 4-layer SFX document the agent fills
    /// in — pure structure, not a preset.
    #[tool(
        name = "scaffold_layered_sfx",
        description = "Create a blank 4-layer SFX document — sub (weight), body (identity), top (air), transient (click) — each on its own mixer layer with a stable id, a band-splitting filter, a one-shot envelope, and a sensible starting gain. The sources are neutral PLACEHOLDERS (sines + noise): swap real sources into each layer with set_param/set_layer, then balance using the per-layer stats every render returns. A correct multi-layer starting point, not a finished sound."
    )]
    pub async fn scaffold_layered_sfx(
        &self,
        params: Parameters<ScaffoldReq>,
    ) -> Result<CallToolResult, String> {
        let req = params.0;
        let base = req.base_freq.unwrap_or(220.0);
        if !(20.0..=20_000.0).contains(&base) {
            return Err(format!("base_freq must be in [20, 20000] Hz, got {base}"));
        }
        let args = serde_json::to_value(&req).map_err(|e| e.to_string())?;
        let name = req
            .name
            .clone()
            .unwrap_or_else(|| "layered_sfx".to_string());
        let graph = scaffold_layered_doc(name, base, req.seed.unwrap_or(0));
        let id = self.store.unique_id(&graph.name);
        let rec = self.build(id, graph, now_secs())?;
        self.jlog("scaffold_layered_sfx", args);
        Ok(sound_result(&rec, true))
    }

    /// Replace the graph for an existing sound and re-render.
    #[tool(
        name = "refine_sound",
        description = "Replace an existing sound's graph with an edited version and re-render. Returns updated analysis."
    )]
    pub async fn refine_sound(
        &self,
        params: Parameters<RefineReq>,
    ) -> Result<CallToolResult, String> {
        let mut req = params.0;
        let existing = self.require(&req.id)?;
        // An omitted version keeps the sound's existing render semantics
        // (refining must never silently re-seed a v1 mixer's noise).
        if req.graph.version.is_none() {
            req.graph.version = existing.graph.version;
        }
        // Likewise keep the sound's kernel revision: refining a legacy
        // (engine-0) sound must not silently anti-alias it. Raise `engine`
        // explicitly to opt into the newer kernels.
        if req.graph.engine.is_none() {
            req.graph.engine = existing.graph.engine;
        }
        let args = serde_json::to_value(&req).map_err(|e| e.to_string())?;
        let RefineReq { id, graph } = req;
        // Build first: a rejected graph must leave history, redo, and the
        // journal untouched, or live and replayed sessions diverge.
        let rec = self.build(id, graph, existing.created_at)?;
        self.checkpoint(&rec.id, &existing.graph);
        self.jlog("refine_sound", args);
        Ok(sound_result(&rec, false))
    }

    /// Fetch the current source graph + metadata for a sound.
    #[tool(
        name = "get_sound",
        description = "Fetch a sound's graph, paths, and analysis."
    )]
    pub async fn get_sound(&self, params: Parameters<IdReq>) -> Result<Json<GetSoundResp>, String> {
        let rec = self.require(&params.0.id)?;
        Ok(Json(GetSoundResp {
            id: rec.id,
            name: rec.name,
            graph: rec.graph,
            wav_path: rec.wav_path.to_string_lossy().into_owned(),
            analysis: rec.analysis,
        }))
    }

    /// List all sounds in the session.
    #[tool(
        name = "list_sounds",
        description = "List all sounds in the session library."
    )]
    pub async fn list_sounds(&self) -> Json<ListResp> {
        let sounds = self
            .store
            .list()
            .into_iter()
            .map(|r| SoundSummary {
                id: r.id,
                name: r.name,
                duration: r.graph.duration,
            })
            .collect();
        Json(ListResp { sounds })
    }

    /// Re-run analysis on demand and return stats + images.
    #[tool(
        name = "analyze",
        description = "Re-run analysis on a sound: levels, loudness, spectral centroid, transients, plus spectrogram and waveform images."
    )]
    pub async fn analyze(&self, params: Parameters<IdReq>) -> Result<CallToolResult, String> {
        let rec = self.require(&params.0.id)?;
        Ok(sound_result(&rec, false))
    }

    /// Write a final game-ready file (optionally re-rendered / re-quantized).
    #[tool(
        name = "export",
        description = "Write a game-ready file: WAV (8/16-bit, smpl loop chunk for loops), FLAC (lossless), or OGG Vorbis (lossy VBR via `quality`, the usual BGM/ambience shipping format). Optional sample-rate override and target_lufs level-matching. dest defaults to the working dir."
    )]
    pub async fn export(&self, params: Parameters<ExportReq>) -> Result<Json<ExportResp>, String> {
        let ExportReq {
            id,
            format,
            bit_depth,
            sample_rate,
            dest,
            target_lufs,
            quality,
        } = params.0;
        if let Some(t) = target_lufs
            && !(-60.0..=0.0).contains(&t)
        {
            return Err(format!("target_lufs must be in [-60, 0] LUFS, got {t}"));
        }
        let rec = self.require(&id)?;

        // Re-render if the caller asked for a different sample rate / loudness.
        let mut graph = rec.graph.clone();
        if let Some(sr) = sample_rate {
            graph.sample_rate = sr;
        }
        if let Some(t) = target_lufs {
            apply_target_lufs(&mut graph, t);
        }
        let product = render::render_product(&graph);
        let bits = bit_depth.unwrap_or(16);

        let dest_path = match dest {
            Some(d) => std::path::PathBuf::from(d),
            None => self
                .store
                .dir()
                .join(format!("{id}_export.{}", format.ext())),
        };
        if let Some(parent) = dest_path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        write_export(
            &dest_path,
            &product,
            &graph,
            format,
            bits,
            quality.unwrap_or(0.5),
        )
        .map_err(|e| format!("failed to write {}: {e}", dest_path.display()))?;
        Ok(Json(ExportResp {
            path: dest_path.to_string_lossy().into_owned(),
        }))
    }

    /// Randomly perturb a sound's graph into a new variant ("nudge until right").
    #[tool(
        name = "mutate_sound",
        description = "Create a new sound by randomly perturbing an existing one's parameters by `amount` (0..1). The original is preserved."
    )]
    pub async fn mutate_sound(
        &self,
        params: Parameters<MutateReq>,
    ) -> Result<CallToolResult, String> {
        let mut req = params.0;
        req.seed = Some(req.seed.unwrap_or_else(now_secs));
        let args = serde_json::to_value(&req).map_err(|e| e.to_string())?;
        let rec = self.require(&req.id)?;
        let graph = vary::mutate(&rec.graph, req.amount, req.seed.unwrap());
        let new_id = self.store.unique_id(&graph.name);
        let new_rec = self.build(new_id, graph, now_secs())?;
        self.jlog("mutate_sound", args);
        Ok(sound_result(&new_rec, true))
    }

    /// Produce N round-robin variations of an existing sound, each slightly
    /// perturbed, so repeated playback (footsteps, impacts, pickups) never
    /// sounds identical.
    #[tool(
        name = "generate_variants",
        description = "Generate `count` round-robin variations (1..32) of an existing sound `id`, each perturbed by `amount` and level-matched to `target_lufs`. Returns a list with per-variant loudness."
    )]
    pub async fn generate_variants(
        &self,
        params: Parameters<VariantsReq>,
    ) -> Result<Json<VariantsResp>, String> {
        let mut req = params.0;
        req.seed = Some(req.seed.unwrap_or_else(now_secs));
        let args = serde_json::to_value(&req).map_err(|e| e.to_string())?;
        if !(1..=32).contains(&req.count) {
            return Err(format!("count must be in 1..=32, got {}", req.count));
        }
        if !(-60.0..=0.0).contains(&req.target_lufs) {
            return Err(format!(
                "target_lufs must be in [-60, 0] LUFS, got {}",
                req.target_lufs
            ));
        }
        let base = self.require(&req.id)?.graph;
        let base_seed = req.seed.unwrap();

        // Generate and validate the whole batch before committing any of it:
        // the journal records this call only after every variant lands, so a
        // rejected graph must not leave half a (journaled-as-whole) set behind.
        let mut graphs = Vec::with_capacity(req.count as usize);
        for k in 0..req.count {
            // Distinct, reproducible per-variant seed.
            let vseed = base_seed
                .wrapping_mul(0x9E37_79B9)
                .wrapping_add(k as u64 + 1);
            let mut graph = vary::mutate(&base, req.amount, vseed);
            // Level-match every take so round-robin playback is consistent.
            graph.normalize = Some(Normalize {
                target_lufs: Some(req.target_lufs),
                ceiling_dbtp: -1.0,
            });
            graph.validate()?;
            graphs.push(graph);
        }
        let mut variants = Vec::with_capacity(graphs.len());
        for graph in graphs {
            let new_id = self.store.unique_id(&graph.name);
            let rec = self.build(new_id, graph, now_secs())?;
            variants.push(variant_summary(&rec));
        }
        self.jlog("generate_variants", args);
        Ok(Json(VariantsResp {
            count: req.count,
            variants,
        }))
    }

    /// Show the addressable structure of a sound: every node's path, type, and
    /// parameters, so the agent knows what it can target before editing.
    #[tool(
        name = "describe_sound",
        description = "The addressing map: every editable node with its path and parameters. Plain sounds list absolute paths (root.inputs[0].freq); mixer sounds list per-layer tables — copy the layer id + the layer-relative path (env.a, notes[3].pitch) straight into set_param/edit_sound, and use set_layer for the mixer fields (gain/pan/at/mute). Call this before editing."
    )]
    pub async fn describe_sound(
        &self,
        params: Parameters<IdReq>,
    ) -> Result<Json<DescribeResp>, String> {
        let rec = self.require(&params.0.id)?;
        let map = edit::describe(&rec.graph);
        Ok(Json(DescribeResp {
            id: rec.id,
            name: rec.name,
            duration: rec.graph.duration,
            nodes: map.nodes,
            layers: map.layers,
            master: map.master,
        }))
    }

    /// Change one parameter (or node) by path and re-render.
    #[tool(
        name = "set_param",
        description = "Change a single parameter (or node) by path without re-sending the whole graph. path e.g. \"root.inputs[0].freq\"; value is a number, a modulator object, or a whole node. Re-renders and returns analysis + images."
    )]
    pub async fn set_param(
        &self,
        params: Parameters<SetParamReq>,
    ) -> Result<CallToolResult, String> {
        let args = serde_json::to_value(&params.0).map_err(|e| e.to_string())?;
        let SetParamReq {
            id,
            layer,
            path,
            value,
        } = params.0;
        let rec = self.require(&id)?;
        let path = match &layer {
            Some(l) => resolve_layer_path(&rec.graph, l, &path)?,
            None => path,
        };
        let graph = edit::apply_ops(&rec.graph, &[EditOp::Set { path, value }])?;
        let new = self.build(id, graph, rec.created_at)?;
        self.checkpoint(&new.id, &rec.graph);
        self.jlog("set_param", args);
        Ok(sound_result(&new, true))
    }

    /// Apply many surgical edits in one re-render.
    #[tool(
        name = "edit_sound",
        description = "Apply many surgical edits to a sound in one re-render. ops is a list of: {op:\"set\", path, value} / {op:\"insert\", path, index?, node} / {op:\"remove\", path, index?}. Prefer over refine_sound for targeted changes — far cheaper than re-sending the whole graph."
    )]
    pub async fn edit_sound(&self, params: Parameters<EditReq>) -> Result<CallToolResult, String> {
        let args = serde_json::to_value(&params.0).map_err(|e| e.to_string())?;
        let EditReq { id, layer, mut ops } = params.0;
        if ops.is_empty() {
            return Err("ops must be non-empty".into());
        }
        let rec = self.require(&id)?;
        if let Some(l) = &layer {
            for op in &mut ops {
                let p = match op {
                    EditOp::Set { path, .. }
                    | EditOp::Insert { path, .. }
                    | EditOp::Remove { path, .. } => path,
                };
                *p = resolve_layer_path(&rec.graph, l, p)?;
            }
        }
        let graph = edit::apply_ops(&rec.graph, &ops)?;
        let new = self.build(id, graph, rec.created_at)?;
        self.checkpoint(&new.id, &rec.graph);
        self.jlog("edit_sound", args);
        Ok(sound_result(&new, true))
    }

    /// Stack a new instrument layer onto a sound (behind `layer { op: "add" }`).
    pub async fn add_layer(
        &self,
        params: Parameters<AddLayerReq>,
    ) -> Result<CallToolResult, String> {
        let args = serde_json::to_value(&params.0).map_err(|e| e.to_string())?;
        let AddLayerReq {
            id,
            layer,
            node,
            gain,
            pan,
            at,
        } = params.0;
        check_layer_slug(&layer)?;
        let rec = self.require(&id)?;
        let mut graph = rec.graph.clone();
        let mut notes = Vec::new();

        if !matches!(graph.root, Node::Tracks { .. }) {
            if !matches!(graph.stereo, Stereo::Mono) {
                return Err(
                    "this sound uses a doc-level stereo treatment (haas/wide); a mixer document \
                     replaces that with per-layer pan — refine_sound it to stereo mode 'mono' \
                     first, then add_layer and pan the layers"
                        .into(),
                );
            }
            // Wrap the existing graph as the first layer, named after the
            // sound. √2 gain exactly compensates the equal-power center pan,
            // so the original keeps its level on the bus.
            let first = if rec.id == layer || rec.id == "master" {
                format!("{}_main", rec.id)
            } else {
                rec.id.clone()
            };
            let old = std::mem::replace(&mut graph.root, Node::Mix { inputs: Vec::new() });
            graph.root = Node::Tracks {
                tracks: vec![Track {
                    id: Some(first.clone()),
                    node: old,
                    pan: 0.0,
                    gain: std::f32::consts::SQRT_2,
                    at: 0.0,
                    mute: false,
                    automation: Vec::new(),
                }],
                master: Vec::new(),
            };
            // The wrap is the moment a sound becomes a mixer document — adopt
            // v2 render semantics (per-layer RNG streams) here, where the
            // re-grain is already announced. Replays re-run this same code,
            // so the upgrade is deterministic.
            graph.version = Some(crate::dsl::SCHEMA_VERSION);
            notes.push(format!(
                "existing graph wrapped as layer '{first}' (gain √2 compensates the equal-power \
                 pan — its level is unchanged; noise textures may re-grain)"
            ));
        } else if graph.effective_version() < 2 {
            notes.push(
                "note: this is a schema-v1 document — its layers share one RNG stream, so \
                 structural changes can re-grain later layers' noise (refine_sound with \
                 version 2 to give every layer its own stream)"
                    .to_string(),
            );
        }

        let Node::Tracks { tracks, .. } = &mut graph.root else {
            unreachable!("wrapped above")
        };
        if tracks.iter().any(|t| t.id.as_deref() == Some(&layer)) {
            return Err(format!(
                "layer '{layer}' already exists in '{id}' (layers: {}) — pick a new id, or edit \
                 the existing layer with set_param/set_layer",
                layer_ids(&graph)
            ));
        }
        tracks.push(Track {
            id: Some(layer.clone()),
            node,
            pan: pan.unwrap_or(0.0),
            gain: gain.unwrap_or(1.0),
            at: at.unwrap_or(0.0),
            mute: false,
            automation: Vec::new(),
        });
        let count = tracks.len();

        let new = self.build(id, graph, rec.created_at)?;
        self.checkpoint(&new.id, &rec.graph);
        self.jlog("add_layer", args);
        let mut res = sound_result(&new, true);
        for n in notes {
            res.content.push(Content::text(n));
        }
        res.content.push(Content::text(format!(
            "layer '{layer}' added ({count} layers: {})",
            layer_ids(&new.graph)
        )));
        Ok(res)
    }

    /// Mixer moves on one layer: fader, pan, time offset, mute (behind
    /// `layer { op: "set" }`).
    pub async fn set_layer(
        &self,
        params: Parameters<SetLayerReq>,
    ) -> Result<CallToolResult, String> {
        let args = serde_json::to_value(&params.0).map_err(|e| e.to_string())?;
        let SetLayerReq {
            id,
            layer,
            gain,
            pan,
            at,
            mute,
        } = params.0;
        if gain.is_none() && pan.is_none() && at.is_none() && mute.is_none() {
            return Err("nothing to set — pass at least one of gain / pan / at / mute".into());
        }
        let rec = self.require(&id)?;
        let mut graph = rec.graph.clone();
        {
            let t = find_layer_mut(&mut graph, &layer)?;
            if let Some(g) = gain {
                t.gain = g;
            }
            if let Some(p) = pan {
                t.pan = p;
            }
            if let Some(a) = at {
                t.at = a;
            }
            if let Some(m) = mute {
                t.mute = m;
            }
        }
        let new = self.build(id, graph, rec.created_at)?;
        self.checkpoint(&new.id, &rec.graph);
        self.jlog("set_layer", args);
        let mut res = sound_result(&new, true);
        if mute == Some(true) {
            res.content.push(Content::text(format!(
                "layer '{layer}' muted — it is off the bus AND off every export until unmuted"
            )));
        }
        Ok(res)
    }

    /// Remove or duplicate a layer (behind `layer { op: "remove" | "duplicate" }`).
    pub async fn layer_ops(
        &self,
        params: Parameters<LayerOpsReq>,
    ) -> Result<CallToolResult, String> {
        let args = serde_json::to_value(&params.0).map_err(|e| e.to_string())?;
        let LayerOpsReq {
            id,
            op,
            layer,
            new_id,
        } = params.0;
        let rec = self.require(&id)?;
        let mut graph = rec.graph.clone();
        // Existence check (with the standard listing error) before mutating.
        find_layer_mut(&mut graph, &layer)?;
        let Node::Tracks { tracks, .. } = &mut graph.root else {
            unreachable!("find_layer_mut verified a tracks root")
        };
        match op {
            LayerOp::Remove => {
                if tracks.len() == 1 {
                    return Err(format!(
                        "'{layer}' is the only layer — a mixer needs at least one; delete or \
                         refine the sound instead"
                    ));
                }
                tracks.retain(|t| t.id.as_deref() != Some(&layer));
            }
            LayerOp::Duplicate => {
                let new_id =
                    new_id.ok_or("duplicate needs new_id — the copy's layer id".to_string())?;
                check_layer_slug(&new_id)?;
                if tracks.iter().any(|t| t.id.as_deref() == Some(&new_id)) {
                    return Err(format!("layer '{new_id}' already exists — pick another id"));
                }
                let pos = tracks
                    .iter()
                    .position(|t| t.id.as_deref() == Some(&layer))
                    .expect("verified above");
                let mut copy = tracks[pos].clone();
                copy.id = Some(new_id);
                tracks.insert(pos + 1, copy);
            }
        }
        let new = self.build(id, graph, rec.created_at)?;
        self.checkpoint(&new.id, &rec.graph);
        self.jlog("layer_ops", args);
        let mut res = sound_result(&new, true);
        res.content.push(Content::text(format!(
            "layers now: {}",
            layer_ids(&new.graph)
        )));
        if new.graph.effective_version() < 2 {
            res.content.push(Content::text(
                "note: schema-v1 document — layers share one RNG stream, so this structural \
                 change may have re-grained later layers' noise"
                    .to_string(),
            ));
        }
        Ok(res)
    }

    /// Add, adjust, or restructure a mixer layer (one tool over the layer ops).
    #[tool(
        name = "layer",
        description = "Operate on a sound's mixer layers, addressed by stable id, and re-render. op=add stacks a new instrument layer (requires `node`; the first add on a plain sound wraps its graph as a level-compensated layer named after the sound). op=set is a mixer move — gain (0..2) / pan (-1..1) / at (start offset s) / mute — graph untouched (muted layers ship out of exports). op=remove deletes a layer (a mixer keeps at least one). op=duplicate copies one (needs new_id; deterministic re-grained noise). One layer per thing you'd fade, pan, time-shift, or analyze separately."
    )]
    pub async fn layer(&self, params: Parameters<LayerReq>) -> Result<CallToolResult, String> {
        let LayerReq {
            id,
            op,
            layer,
            node,
            gain,
            pan,
            at,
            mute,
            new_id,
        } = params.0;
        match op {
            LayerKind::Add => {
                let node =
                    node.ok_or_else(|| "layer op 'add' requires a `node` graph".to_string())?;
                self.add_layer(Parameters(AddLayerReq {
                    id,
                    layer,
                    node,
                    gain,
                    pan,
                    at,
                }))
                .await
            }
            LayerKind::Set => {
                self.set_layer(Parameters(SetLayerReq {
                    id,
                    layer,
                    gain,
                    pan,
                    at,
                    mute,
                }))
                .await
            }
            LayerKind::Remove => {
                self.layer_ops(Parameters(LayerOpsReq {
                    id,
                    op: LayerOp::Remove,
                    layer,
                    new_id,
                }))
                .await
            }
            LayerKind::Duplicate => {
                self.layer_ops(Parameters(LayerOpsReq {
                    id,
                    op: LayerOp::Duplicate,
                    layer,
                    new_id,
                }))
                .await
            }
        }
    }

    /// Compare two sounds: metric deltas + a similarity score (convergence aid).
    #[tool(
        name = "compare_sounds",
        description = "Compare two sounds and report metric deltas (b - a: centroid/brightness, LUFS, peak, crest, attack, decay, duration) plus a 0..1 similarity score. Use it to converge a sound toward a reference."
    )]
    pub async fn compare_sounds(
        &self,
        params: Parameters<CompareReq>,
    ) -> Result<Json<CompareResp>, String> {
        let CompareReq { a, b } = params.0;
        let ra = self.require(&a)?;
        let rb = self.require(&b)?;
        let (x, y) = (&ra.analysis, &rb.analysis);
        let centroid_delta_hz = y.spectral_centroid_hz - x.spectral_centroid_hz;
        let lufs_delta = y.loudness_lufs - x.loudness_lufs;
        let peak_delta_db = y.peak_dbfs - x.peak_dbfs;
        let crest_delta_db = y.crest_factor_db - x.crest_factor_db;
        let attack_delta_ms = y.attack_time_ms - x.attack_time_ms;
        let decay_delta_ms = y.decay_time_ms - x.decay_time_ms;
        let duration_delta_ms = (y.duration_secs - x.duration_secs) * 1000.0;
        // Normalised, weighted distance → similarity in (0, 1].
        let dist = centroid_delta_hz.abs() / 2000.0
            + lufs_delta.abs() / 6.0
            + crest_delta_db.abs() / 6.0
            + attack_delta_ms.abs() / 50.0
            + duration_delta_ms.abs() / 300.0;
        Ok(Json(CompareResp {
            a,
            b,
            similarity: round2((-dist).exp()),
            centroid_delta_hz: round2(centroid_delta_hz),
            lufs_delta: round2(lufs_delta),
            peak_delta_db: round2(peak_delta_db),
            crest_delta_db: round2(crest_delta_db),
            attack_delta_ms: round2(attack_delta_ms),
            decay_delta_ms: round2(decay_delta_ms),
            duration_delta_ms: round2(duration_delta_ms),
        }))
    }

    /// Grade a sound against archetype targets + the universal ship checklist —
    /// the automated "Review" half of a review → polish → review loop.
    #[tool(
        name = "review_sound",
        description = "Grade a sound against its archetype's targets (attack/centroid/crest/duration) and the universal ship checklist (clipping, true-peak, head/tail silence, onset count, loop seam). Returns PASS/WARN/FAIL findings, each with the measured value, the target, and the concrete fix to try — a deterministic, reproducible critique to drive an iterative polish loop. Pass an archetype (laser/coin/jump/impact/ui/ambience/bgm) for the full review, or omit it for the universal checks only."
    )]
    pub async fn review_sound(
        &self,
        params: Parameters<ReviewReq>,
    ) -> Result<Json<Review>, String> {
        let ReviewReq { id, archetype } = params.0;
        let rec = self.require(&id)?;
        // The loop-seam check needs the rendered samples (the stored Analysis
        // doesn't carry it); render once, only when the document loops.
        let seam = if matches!(rec.graph.playback, Playback::Loop { .. }) {
            Some(render::loop_seam_db(
                &render::render_product(&rec.graph).mono,
            ))
        } else {
            None
        };
        Ok(Json(review::review(
            &rec.graph,
            &rec.analysis,
            archetype,
            seam,
        )))
    }

    /// Create in-between sounds along the line between two designs.
    #[tool(
        name = "morph_sounds",
        description = "Create `steps` in-between sounds interpolating two SAME-SHAPED graphs (a at t=0 toward b at t=1) — charge tiers, damage levels, a soft-to-harsh series. Numeric params lerp; note names lerp in Hz; structural differences fail with a clear message."
    )]
    pub async fn morph_sounds(
        &self,
        params: Parameters<MorphReq>,
    ) -> Result<Json<VariantsResp>, String> {
        let args = serde_json::to_value(&params.0).map_err(|e| e.to_string())?;
        let MorphReq { a, b, steps } = params.0;
        if !(1..=10).contains(&steps) {
            return Err(format!("steps must be in 1..=10, got {steps}"));
        }
        let ra = self.require(&a)?;
        let rb = self.require(&b)?;
        // Morph (and validate) the whole series before committing any of it —
        // a structural mismatch must not leave a partial, unjournaled set.
        let mut graphs = Vec::with_capacity(steps as usize);
        for k in 1..=steps {
            let t = k as f32 / (steps + 1) as f32;
            let mut graph = edit::morph(&ra.graph, &rb.graph, t)?;
            graph.name = format!("{}_to_{}_{k}", ra.id, rb.id);
            graphs.push(graph);
        }
        let mut variants = Vec::with_capacity(graphs.len());
        for graph in graphs {
            let id = self.store.unique_id(&graph.name);
            let rec = self.build(id, graph, now_secs())?;
            variants.push(variant_summary(&rec));
        }
        self.jlog("morph_sounds", args);
        Ok(Json(VariantsResp {
            count: steps,
            variants,
        }))
    }

    /// Musically-coherent round-robin takes (one pitch shift + one level trim).
    #[tool(
        name = "humanize",
        description = "Create `count` round-robin takes with ONE coherent pitch shift (max pitch_cents) and ONE level trim (max gain_db) per take — the variation a real performer produces between repeats. Unlike mutate_sound, identity parameters (envelopes, filters, structure) stay untouched, so a footstep stays exactly that footstep."
    )]
    pub async fn humanize(
        &self,
        params: Parameters<HumanizeReq>,
    ) -> Result<Json<VariantsResp>, String> {
        let mut req = params.0;
        req.seed = Some(req.seed.unwrap_or_else(now_secs));
        let args = serde_json::to_value(&req).map_err(|e| e.to_string())?;
        if !(1..=16).contains(&req.count) {
            return Err(format!("count must be in 1..=16, got {}", req.count));
        }
        if !(0.0..=200.0).contains(&req.pitch_cents) {
            return Err(format!(
                "pitch_cents must be in [0, 200], got {}",
                req.pitch_cents
            ));
        }
        if !(0.0..=12.0).contains(&req.gain_db) {
            return Err(format!("gain_db must be in [0, 12], got {}", req.gain_db));
        }
        let rec = self.require(&req.id)?;
        let base_seed = req.seed.unwrap();
        // Validate the whole batch of takes before committing any of it.
        let mut graphs = Vec::with_capacity(req.count as usize);
        for k in 0..req.count {
            let vseed = base_seed
                .wrapping_mul(0x9E37_79B9)
                .wrapping_add(k as u64 + 1);
            let graph = vary::humanize(&rec.graph, req.pitch_cents, req.gain_db, vseed);
            graph.validate()?;
            graphs.push(graph);
        }
        let mut variants = Vec::with_capacity(graphs.len());
        for graph in graphs {
            let new_id = self.store.unique_id(&graph.name);
            let nrec = self.build(new_id, graph, now_secs())?;
            variants.push(variant_summary(&nrec));
        }
        self.jlog("humanize", args);
        Ok(Json(VariantsResp {
            count: req.count,
            variants,
        }))
    }

    /// Turn a sound into a seamless loop (ambience / drone / BGM).
    #[tool(
        name = "make_loop",
        description = "Turn a sound into a seamless loop: extract [start_secs, end_secs) and equal-power crossfade the tail onto the head so it repeats with no click. Sets playback=loop (the exported WAV carries a smpl loop chunk engines read). Returns analysis + the loop-seam discontinuity in dB (lower is cleaner)."
    )]
    pub async fn make_loop(
        &self,
        params: Parameters<MakeLoopReq>,
    ) -> Result<CallToolResult, String> {
        let args = serde_json::to_value(&params.0).map_err(|e| e.to_string())?;
        let MakeLoopReq {
            id,
            crossfade_secs,
            start_secs,
            end_secs,
        } = params.0;
        let rec = self.require(&id)?;
        let mut graph = rec.graph.clone();
        graph.playback = Playback::Loop {
            start_secs,
            end_secs,
            crossfade_secs,
        };
        graph.validate()?;
        let (new, product) = self.build_product(id, graph, rec.created_at)?;
        self.checkpoint(&new.id, &rec.graph);
        self.jlog("make_loop", args);
        // Report how clean the resulting loop seam is.
        let looped = product.mono;
        let seam = render::loop_seam_db(&looped);
        let mut res = sound_result(&new, true);
        res.content.push(Content::text(format!(
            "loop: {:.3}s body • seam discontinuity {seam:.1} dB (lower = cleaner; redesign the graph or raise crossfade_secs if high)",
            looped.len() as f32 / new.graph.sample_rate as f32,
        )));
        Ok(res)
    }

    /// Revert a sound to its previous graph (kept as the implementation behind
    /// `history { op: "undo" }`; journals "undo_sound" so sessions replay).
    pub async fn undo_sound(&self, params: Parameters<IdReq>) -> Result<CallToolResult, String> {
        let args = serde_json::to_value(&params.0).map_err(|e| e.to_string())?;
        let id = params.0.id;
        let rec = self.require(&id)?;
        let prev = self
            .store
            .pop_history(&id)
            .ok_or_else(|| format!("nothing to undo for '{id}'"))?;
        self.store.push_redo(&id, &rec.graph);
        let new = self.build(id, prev, rec.created_at)?;
        self.jlog("undo_sound", args);
        Ok(sound_result(&new, true))
    }

    /// Re-apply the most recently undone edit (behind `history { op: "redo" }`).
    pub async fn redo_sound(&self, params: Parameters<IdReq>) -> Result<CallToolResult, String> {
        let args = serde_json::to_value(&params.0).map_err(|e| e.to_string())?;
        let id = params.0.id;
        let rec = self.require(&id)?;
        let next = self
            .store
            .pop_redo(&id)
            .ok_or_else(|| format!("nothing to redo for '{id}'"))?;
        self.store.push_history(&id, &rec.graph);
        let new = self.build(id, next, rec.created_at)?;
        self.jlog("redo_sound", args);
        Ok(sound_result(&new, true))
    }

    /// Report a sound's undo/redo depths (behind `history { op: "status" }`).
    pub async fn history(&self, params: Parameters<IdReq>) -> Result<Json<HistoryResp>, String> {
        let id = params.0.id;
        self.require(&id)?;
        let (undo_depth, redo_depth) = self.store.history_depths(&id);
        Ok(Json(HistoryResp {
            id,
            undo_depth,
            redo_depth,
        }))
    }

    /// Step through or inspect a sound's revision history.
    #[tool(
        name = "history",
        description = "Per-sound revision history. op=status (default) reports undo/redo depths; op=undo reverts to the previous graph (the undone state moves to the redo stack); op=redo re-applies the last undone edit. Every refine/set_param/edit_sound/make_loop pushes a revision (bounded 100-step)."
    )]
    pub async fn history_op(
        &self,
        params: Parameters<HistoryReq>,
    ) -> Result<CallToolResult, String> {
        let HistoryReq { id, op } = params.0;
        match op {
            HistoryOp::Undo => self.undo_sound(Parameters(IdReq { id })).await,
            HistoryOp::Redo => self.redo_sound(Parameters(IdReq { id })).await,
            HistoryOp::Status => {
                let h = self.history(Parameters(IdReq { id })).await?.0;
                Ok(CallToolResult::success(vec![Content::text(format!(
                    "{}: undo_depth {}, redo_depth {}",
                    h.id, h.undo_depth, h.redo_depth
                ))]))
            }
        }
    }

    /// Create a named sound bank (behind `bank { op: "create" }`).
    pub async fn create_bank(
        &self,
        params: Parameters<CreateBankReq>,
    ) -> Result<Json<Bank>, String> {
        let args = serde_json::to_value(&params.0).map_err(|e| e.to_string())?;
        let name = params.0.name;
        let id = self.store.unique_bank_id(&name);
        let bank = Bank {
            id,
            name,
            members: Vec::new(),
        };
        self.store
            .put_bank(bank.clone())
            .map_err(|e| e.to_string())?;
        self.jlog("create_bank", args);
        Ok(Json(bank))
    }

    /// Add or update a sound's membership in a bank (behind `bank { op: "add" }`).
    pub async fn add_to_bank(
        &self,
        params: Parameters<AddToBankReq>,
    ) -> Result<Json<Bank>, String> {
        let args = serde_json::to_value(&params.0).map_err(|e| e.to_string())?;
        let AddToBankReq {
            bank_id,
            sound_id,
            category,
            rr_group,
        } = params.0;
        self.require(&sound_id)?;
        let mut bank = self
            .store
            .get_bank(&bank_id)
            .ok_or_else(|| format!("no bank with id '{bank_id}'"))?;
        if let Some(m) = bank.members.iter_mut().find(|m| m.sound_id == sound_id) {
            m.category = category;
            m.rr_group = rr_group;
        } else {
            bank.members.push(BankMember {
                sound_id,
                category,
                rr_group,
            });
        }
        self.store
            .put_bank(bank.clone())
            .map_err(|e| e.to_string())?;
        self.jlog("add_to_bank", args);
        Ok(Json(bank))
    }

    /// List all sound banks (behind `bank { op: "list" }`).
    pub async fn list_banks(&self) -> Json<BanksResp> {
        Json(BanksResp {
            banks: self.store.list_banks(),
        })
    }

    /// Create a bank, add a sound to one, or list them.
    #[tool(
        name = "bank",
        description = "Manage sound banks (engine-facing packs). op=create makes a named bank (requires `name`; returns it with a stable id). op=add adds/updates a sound's membership (requires `bank_id` + `sound_id`; optional `category` and `rr_group` round-robin group). op=list returns every bank and its members. Export a bank with export_pack."
    )]
    pub async fn bank(&self, params: Parameters<BankReq>) -> Result<Json<BankResp>, String> {
        let BankReq {
            op,
            name,
            bank_id,
            sound_id,
            category,
            rr_group,
        } = params.0;
        match op {
            BankKind::Create => {
                let name = name.ok_or_else(|| "bank op 'create' requires a `name`".to_string())?;
                let bank = self
                    .create_bank(Parameters(CreateBankReq { name }))
                    .await?
                    .0;
                Ok(Json(BankResp {
                    bank: Some(bank),
                    banks: None,
                }))
            }
            BankKind::Add => {
                let bank_id =
                    bank_id.ok_or_else(|| "bank op 'add' requires a `bank_id`".to_string())?;
                let sound_id =
                    sound_id.ok_or_else(|| "bank op 'add' requires a `sound_id`".to_string())?;
                let bank = self
                    .add_to_bank(Parameters(AddToBankReq {
                        bank_id,
                        sound_id,
                        category,
                        rr_group,
                    }))
                    .await?
                    .0;
                Ok(Json(BankResp {
                    bank: Some(bank),
                    banks: None,
                }))
            }
            BankKind::List => Ok(Json(BankResp {
                bank: None,
                banks: Some(self.list_banks().await.0.banks),
            })),
        }
    }

    /// Export a bank's sounds + a manifest (behind `export_pack { bank_id }`).
    pub async fn export_bank(
        &self,
        params: Parameters<ExportBankReq>,
    ) -> Result<Json<PackResp>, String> {
        let ExportBankReq {
            bank_id,
            dest,
            by_category,
            target_lufs,
            format,
            quality,
            engine,
        } = params.0;
        let bank = self
            .store
            .get_bank(&bank_id)
            .ok_or_else(|| format!("no bank with id '{bank_id}'"))?;
        self.write_pack(
            &bank.members,
            &dest,
            PackOptions {
                by_category,
                target_lufs,
                format,
                quality: quality.unwrap_or(0.5),
                engine,
            },
        )
    }

    /// Export every sound in the library + a manifest (behind `export_pack`
    /// with no `bank_id`).
    pub async fn export_all(
        &self,
        params: Parameters<ExportAllReq>,
    ) -> Result<Json<PackResp>, String> {
        let ExportAllReq {
            dest,
            target_lufs,
            format,
            quality,
            engine,
        } = params.0;
        let members: Vec<BankMember> = self
            .store
            .list()
            .into_iter()
            .map(|r| BankMember {
                sound_id: r.id,
                category: None,
                rr_group: None,
            })
            .collect();
        self.write_pack(
            &members,
            &dest,
            PackOptions {
                by_category: false,
                target_lufs,
                format,
                quality: quality.unwrap_or(0.5),
                engine,
            },
        )
    }

    /// Export a pack — one bank, or the whole library.
    #[tool(
        name = "export_pack",
        description = "Export a pack into `dest` plus a sounds.json manifest (id, file, category, rr_group, duration, loudness, peak, channels) so a game engine wires the whole set with no hand-listing. With `bank_id`, exports that bank (by_category lays sounds into per-category subfolders); omit `bank_id` to export the entire library. Optional target_lufs level-matches the set; engine: godot | unity | bevy also emits .import / .meta sidecars or a sonarium_sounds.rs module."
    )]
    pub async fn export_pack(
        &self,
        params: Parameters<ExportPackReq>,
    ) -> Result<Json<PackResp>, String> {
        let ExportPackReq {
            bank_id,
            dest,
            by_category,
            target_lufs,
            format,
            quality,
            engine,
        } = params.0;
        match bank_id {
            Some(bank_id) => {
                self.export_bank(Parameters(ExportBankReq {
                    bank_id,
                    dest,
                    by_category,
                    target_lufs,
                    format,
                    quality,
                    engine,
                }))
                .await
            }
            None => {
                self.export_all(Parameters(ExportAllReq {
                    dest,
                    target_lufs,
                    format,
                    quality,
                    engine,
                }))
                .await
            }
        }
    }

    /// Snapshot the session journal to a portable session file.
    #[tool(
        name = "save_session",
        description = "Snapshot the session journal (every mutating tool call so far) to a portable .jsonl session file. Replaying that file into a fresh working directory reproduces the whole session — same sounds, same banks, byte-identical audio."
    )]
    pub async fn save_session(
        &self,
        params: Parameters<SaveSessionReq>,
    ) -> Result<Json<SaveSessionResp>, String> {
        let steps = journal::read_steps(self.journal.path())
            .map_err(|e| format!("nothing to save: {e}"))?;
        if steps.is_empty() {
            return Err("nothing to save: the session journal is empty".into());
        }
        let dest = params
            .0
            .dest
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| self.store.dir().join("session_save.jsonl"));
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        std::fs::copy(self.journal.path(), &dest).map_err(|e| format!("copy session file: {e}"))?;
        Ok(Json(SaveSessionResp {
            path: dest.to_string_lossy().into_owned(),
            steps: steps.len() as u32,
        }))
    }

    /// Re-apply a saved session file into the current session.
    #[tool(
        name = "replay_session",
        description = "Replay a saved session file into a FRESH session: re-apply its recorded tool calls in order (same seeds ⇒ byte-identical audio). Fails if the working directory already holds sounds, banks, or a journal — ids derive from sound names, so replaying over existing content would silently edit the wrong sounds."
    )]
    pub async fn replay_session(
        &self,
        params: Parameters<ReplaySessionReq>,
    ) -> Result<Json<ReplaySessionResp>, String> {
        use std::sync::atomic::Ordering;
        let path = std::path::PathBuf::from(params.0.path);
        let steps = journal::read_steps(&path).map_err(|e| e.to_string())?;
        // Replay only into a pristine session. Ids derive from sound names, so
        // an existing sound with the same name would shift the ids a replayed
        // author_sound mints — and every later id-addressed step would then
        // silently edit the pre-existing sound instead of the replayed copy.
        if !self.store.list().is_empty()
            || !self.store.list_banks().is_empty()
            || !self.journal.is_empty()
        {
            return Err(
                "replay_session requires a fresh session: the working directory already \
                 contains sounds, banks, or a session journal. Start the server with an \
                 empty SONARIUM_WORKDIR and replay there."
                    .into(),
            );
        }
        // Suppress per-tool journaling and copy the replayed steps in verbatim
        // instead: the live journal then exactly mirrors the applied steps, so
        // a later save_session reproduces this session without duplication.
        self.replaying.store(true, Ordering::SeqCst);
        for (i, step) in steps.iter().enumerate() {
            if let Err(e) = self.apply_step(step).await {
                self.replaying.store(false, Ordering::SeqCst);
                return Err(format!("step {} ({}): {e}", i + 1, step.tool));
            }
            if let Err(e) = self.journal.append(&step.tool, &step.args) {
                tracing::warn!("session journal write failed during replay: {e}");
            }
        }
        self.replaying.store(false, Ordering::SeqCst);
        Ok(Json(ReplaySessionResp {
            applied: steps.len() as u32,
        }))
    }

    /// Dispatch one journaled step back through the tool it recorded.
    async fn apply_step(&self, step: &journal::Step) -> Result<(), String> {
        fn de<T: serde::de::DeserializeOwned>(v: &serde_json::Value) -> Result<T, String> {
            serde_json::from_value(v.clone()).map_err(|e| format!("bad args: {e}"))
        }
        let a = &step.args;
        match step.tool.as_str() {
            "author_sound" => self.author_sound(Parameters(de(a)?)).await.map(drop),
            "scaffold_layered_sfx" => self
                .scaffold_layered_sfx(Parameters(de(a)?))
                .await
                .map(drop),
            "refine_sound" => self.refine_sound(Parameters(de(a)?)).await.map(drop),
            "set_param" => self.set_param(Parameters(de(a)?)).await.map(drop),
            "edit_sound" => self.edit_sound(Parameters(de(a)?)).await.map(drop),
            "add_layer" => self.add_layer(Parameters(de(a)?)).await.map(drop),
            "set_layer" => self.set_layer(Parameters(de(a)?)).await.map(drop),
            "layer_ops" => self.layer_ops(Parameters(de(a)?)).await.map(drop),
            "mutate_sound" => self.mutate_sound(Parameters(de(a)?)).await.map(drop),
            "generate_variants" => self.generate_variants(Parameters(de(a)?)).await.map(drop),
            "morph_sounds" => self.morph_sounds(Parameters(de(a)?)).await.map(drop),
            "humanize" => self.humanize(Parameters(de(a)?)).await.map(drop),
            "make_loop" => self.make_loop(Parameters(de(a)?)).await.map(drop),
            "undo_sound" => self.undo_sound(Parameters(de(a)?)).await.map(drop),
            "redo_sound" => self.redo_sound(Parameters(de(a)?)).await.map(drop),
            "create_bank" => self.create_bank(Parameters(de(a)?)).await.map(drop),
            "add_to_bank" => self.add_to_bank(Parameters(de(a)?)).await.map(drop),
            other => Err(format!("not a replayable tool: '{other}'")),
        }
    }

    /// Render `members` into `dest` and write a `sounds.json` manifest.
    fn write_pack(
        &self,
        members: &[BankMember],
        dest: &str,
        opts: PackOptions,
    ) -> Result<Json<PackResp>, String> {
        let PackOptions {
            by_category,
            target_lufs,
            format,
            quality,
            engine,
        } = opts;
        let dest = std::path::PathBuf::from(dest);
        std::fs::create_dir_all(&dest).map_err(|e| format!("create {}: {e}", dest.display()))?;

        let mut entries = Vec::new();
        for m in members {
            let Some(rec) = self.store.get(&m.sound_id) else {
                continue;
            };
            let mut graph = rec.graph.clone();
            if let Some(t) = target_lufs {
                apply_target_lufs(&mut graph, t);
            }
            let product = render::render_product(&graph);

            // Lay out the file (optionally under a category subfolder).
            let ext = format.ext();
            let (rel, abs) = match (by_category, m.category.as_deref()) {
                (true, Some(cat)) => {
                    let sub = slugify(cat);
                    let sub = if sub.is_empty() {
                        "misc".to_string()
                    } else {
                        sub
                    };
                    std::fs::create_dir_all(dest.join(&sub))
                        .map_err(|e| format!("create {sub}: {e}"))?;
                    (
                        format!("{sub}/{}.{ext}", rec.id),
                        dest.join(&sub).join(format!("{}.{ext}", rec.id)),
                    )
                }
                _ => (
                    format!("{}.{ext}", rec.id),
                    dest.join(format!("{}.{ext}", rec.id)),
                ),
            };
            write_export(&abs, &product, &graph, format, 16, quality)
                .map_err(|e| format!("write {}: {e}", abs.display()))?;

            let channels = if matches!(graph.root, Node::Tracks { .. })
                || !matches!(graph.stereo, Stereo::Mono)
            {
                2
            } else {
                1
            };
            entries.push(ManifestEntry {
                id: rec.id.clone(),
                name: rec.name.clone(),
                file: rel,
                category: m.category.clone(),
                rr_group: m.rr_group.clone(),
                looped: matches!(graph.playback, Playback::Loop { .. }),
                duration_ms: (product.mono.len() as f32 / graph.sample_rate as f32 * 1000.0).round()
                    as u32,
                sample_rate: graph.sample_rate,
                channels,
                lufs: round2(analysis::loudness_lufs(&product.mono)),
                peak_dbfs: round2(dbfs(peak_abs(&product.mono))),
                true_peak_dbfs: round2(dbfs(analysis::true_peak(&product.mono))),
            });
        }

        let manifest = json!({
            "count": entries.len(),
            "sounds": entries,
        });
        let manifest_path = dest.join("sounds.json");
        std::fs::write(
            &manifest_path,
            serde_json::to_string_pretty(&manifest).map_err(|e| e.to_string())?,
        )
        .map_err(|e| format!("write manifest: {e}"))?;

        // Engine integration files (sidecars / generated module).
        let engine_files = match engine {
            Some(target) => {
                engines::emit(target, &dest, &entries).map_err(|e| format!("engine emit: {e}"))?
            }
            None => Vec::new(),
        };

        Ok(Json(PackResp {
            count: entries.len() as u32,
            manifest_path: manifest_path.to_string_lossy().into_owned(),
            entries,
            engine_files,
        }))
    }

    /// Shared author/refine pipeline: validate → render → write WAV → analyze → store.
    fn build(&self, id: String, graph: SoundDoc, created_at: u64) -> Result<Record, String> {
        self.build_product(id, graph, created_at)
            .map(|(rec, _)| rec)
    }

    /// [`build`], also handing back the render product so callers that need
    /// the finished samples (loop seam reporting) never render twice.
    fn build_product(
        &self,
        id: String,
        mut graph: SoundDoc,
        created_at: u64,
    ) -> Result<(Record, render::RenderProduct), String> {
        // Mixer tracks without ids get deterministic ones here (the one
        // chokepoint every graph passes through), so persisted documents are
        // always layer-addressable and replays mint identical ids.
        graph.ensure_track_ids();
        graph.validate()?;
        let product = render::render_product(&graph);

        let wav_path = self.store.wav_path(&id);
        write_render(&wav_path, &product, &graph, 16)
            .map_err(|e| format!("render write failed: {e}"))?;

        let png_path = self.store.png_path(&id);
        let mut analysis = analysis::analyze(&product.mono, graph.sample_rate, &png_path)
            .map_err(|e| format!("analysis failed: {e}"))?;
        analysis.layers = product.layers.clone();

        let rec = Record {
            id,
            name: graph.name.clone(),
            graph,
            wav_path,
            analysis,
            created_at,
        };
        self.store
            .put(rec.clone())
            .map_err(|e| format!("store failed: {e}"))?;
        Ok((rec, product))
    }

    /// Fetch a record or fail with the standard message.
    fn require(&self, id: &str) -> Result<Record, String> {
        self.store
            .get(id)
            .ok_or_else(|| format!("no sound with id '{id}'"))
    }

    /// Push the current graph as an undo revision and invalidate redo (the
    /// standard bookkeeping before any destructive change).
    fn checkpoint(&self, id: &str, graph: &SoundDoc) {
        self.store.push_history(id, graph);
        self.store.clear_redo(id);
    }

    /// Record a successful mutating call in the session journal. Best-effort:
    /// the tool already succeeded, so a journal write failure is logged rather
    /// than turned into a tool error.
    fn jlog(&self, tool: &str, args: serde_json::Value) {
        // During replay the steps are copied into the journal verbatim by
        // replay_session itself — re-journaling here would double them.
        if self.replaying.load(std::sync::atomic::Ordering::SeqCst) {
            return;
        }
        if let Err(e) = self.journal.append(tool, &args) {
            tracing::warn!("session journal write failed for {tool}: {e}");
        }
    }
}

/// Peak absolute sample value.
fn peak_abs(samples: &[f32]) -> f32 {
    samples.iter().fold(0.0f32, |m, &x| m.max(x.abs()))
}

/// The comma-joined layer ids of a mixer document (for listing in errors and
/// confirmations).
fn layer_ids(graph: &SoundDoc) -> String {
    let Node::Tracks { tracks, .. } = &graph.root else {
        return String::new();
    };
    tracks
        .iter()
        .enumerate()
        .map(|(i, t)| t.id.clone().unwrap_or_else(|| format!("layer_{i}")))
        .collect::<Vec<_>>()
        .join(", ")
}

/// Layer ids are short slugs so they survive round-trips through tool calls.
fn check_layer_slug(layer: &str) -> Result<(), String> {
    if layer.is_empty()
        || !layer
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
    {
        let hint = slugify(layer);
        return Err(format!(
            "layer ids are short slugs (a-z, 0-9, _), got '{layer}'{}",
            if hint.is_empty() {
                String::new()
            } else {
                format!(" — try '{hint}'")
            }
        ));
    }
    if layer == "master" {
        return Err(
            "'master' is the master chain, not a layer — address its processors with plain \
             paths like 'root.master[0].room'"
                .into(),
        );
    }
    Ok(())
}

/// Find a layer by id, with the standard teaching error.
fn find_layer_mut<'a>(graph: &'a mut SoundDoc, layer: &str) -> Result<&'a mut Track, String> {
    let name = graph.name.clone();
    let listing = layer_ids(graph);
    let Node::Tracks { tracks, .. } = &mut graph.root else {
        return Err(format!(
            "'{name}' has no layers yet — add_layer turns it into a mixer document"
        ));
    };
    tracks
        .iter_mut()
        .find(|t| t.id.as_deref() == Some(layer))
        .ok_or_else(|| format!("no layer '{layer}' in '{name}'; layers: {listing}"))
}

/// Resolve a layer-relative path (`env.a`, `notes[3].pitch`) to the absolute
/// document path `root.tracks[<i>].node.<path>`.
fn resolve_layer_path(graph: &SoundDoc, layer: &str, path: &str) -> Result<String, String> {
    if layer == "master" {
        return Err(
            "'master' is not a layer — address master processors with plain paths like \
             'root.master[0].room' (no layer arg)"
                .into(),
        );
    }
    // Match the FIRST path segment so 'gain[0]' / 'mute.x' get the teaching
    // error too — while nested uses like 'notes[3].gain' stay untouched.
    let first_seg = path.split(['.', '[']).next().unwrap_or(path);
    if matches!(first_seg, "gain" | "pan" | "at" | "mute") {
        return Err(format!(
            "'{path}' is a mixer field — use set_layer; layer paths address the instrument \
             graph (e.g. 'env.a', 'notes[3].pitch', 'stages[1].cutoff')"
        ));
    }
    if first_seg == "id" {
        return Err(
            "layer ids are immutable addresses — use layer_ops duplicate + remove to rename".into(),
        );
    }
    if path == "root" || path.starts_with("root.") || path.starts_with("root[") {
        return Err(
            "layer paths are relative to the layer's node — drop the 'root.tracks[..]' prefix \
             (e.g. 'inputs[0].freq', or '' for the node itself)"
                .into(),
        );
    }
    let Node::Tracks { tracks, .. } = &graph.root else {
        return Err(format!(
            "'{}' has no layers — drop the layer arg, or add_layer first",
            graph.name
        ));
    };
    let i = tracks
        .iter()
        .position(|t| t.id.as_deref() == Some(layer))
        .ok_or_else(|| {
            format!(
                "no layer '{layer}' in '{}'; layers: {}",
                graph.name,
                layer_ids(graph)
            )
        })?;
    // parse_path skips empty segments, so an empty layer path addresses the
    // layer's node itself.
    Ok(format!("root.tracks[{i}].node.{path}"))
}

/// The level-reporting row shared by every multi-sound generator.
fn variant_summary(rec: &Record) -> VariantSummary {
    VariantSummary {
        id: rec.id.clone(),
        name: rec.name.clone(),
        wav_path: rec.wav_path.to_string_lossy().into_owned(),
        loudness_lufs: rec.analysis.loudness_lufs,
        peak_dbfs: rec.analysis.peak_dbfs,
        spectral_centroid_hz: rec.analysis.spectral_centroid_hz,
    }
}

/// Round to 2 decimals for tidy manifest numbers.
fn round2(x: f32) -> f32 {
    (x * 100.0).round() / 100.0
}

/// Force a loudness target onto a graph's output stage, preserving any
/// explicitly chosen true-peak ceiling.
fn apply_target_lufs(graph: &mut SoundDoc, target: f32) {
    let ceiling_dbtp = graph.normalize.map(|n| n.ceiling_dbtp).unwrap_or(-1.0);
    graph.normalize = Some(Normalize {
        target_lufs: Some(target),
        ceiling_dbtp,
    });
}

/// Rebuild the in-memory index from graphs persisted on disk so a restarted
/// server still sees previously authored sounds (and banks). Each graph is the
/// source of truth: it is re-rendered and re-analyzed. Invalid / unreadable
/// graphs are skipped. Returns the number of sounds restored.
pub fn rehydrate(store: &Store) -> usize {
    store.load_banks();
    let mut restored = 0;
    for (id, mut graph, created_at) in store.list_graph_files() {
        // Pre-layering mixer documents have no track ids — backfill exactly
        // like build does, so describe/set_layer addresses resolve after a
        // restart. (Positional + deterministic; v1 rendering ignores ids.)
        graph.ensure_track_ids();
        if graph.validate().is_err() {
            continue;
        }
        let product = render::render_product(&graph);
        let wav_path = store.wav_path(&id);
        if write_render(&wav_path, &product, &graph, 16).is_err() {
            continue;
        }
        let png_path = store.png_path(&id);
        let Ok(mut analysis) = analysis::analyze(&product.mono, graph.sample_rate, &png_path)
        else {
            continue;
        };
        analysis.layers = product.layers.clone();
        let rec = Record {
            id: id.clone(),
            name: graph.name.clone(),
            graph,
            wav_path,
            analysis,
            created_at,
        };
        if store.put(rec).is_ok() {
            restored += 1;
        }
    }
    restored
}

/// Write a finished render to disk: the true stereo bus for mixer documents,
/// otherwise the mono buffer with the graph's stereo treatment applied when it
/// isn't `Mono` (then the file is interleaved stereo).
fn write_render(
    path: &std::path::Path,
    product: &render::RenderProduct,
    graph: &SoundDoc,
    bits: u16,
) -> anyhow::Result<()> {
    let mono = &product.mono;
    // A mixer document is true stereo: write the panned bus, not the mid.
    if let Some((l, r)) = &product.stereo {
        audio::write_wav_stereo(path, l, r, graph.sample_rate, bits)?;
        if matches!(graph.playback, Playback::Loop { .. }) && !mono.is_empty() {
            audio::append_smpl_loop(path, graph.sample_rate, 0, mono.len() as u32 - 1)?;
        }
        return Ok(());
    }
    if matches!(graph.stereo, Stereo::Mono) {
        audio::write_wav(path, mono, graph.sample_rate, bits)?;
    } else {
        let (l, r) = render::stereoize(mono, graph.stereo, graph.sample_rate);
        audio::write_wav_stereo(path, &l, &r, graph.sample_rate, bits)?;
    }
    // A looped sound carries a `smpl` chunk spanning the whole (already
    // seam-crossfaded) buffer, so engines loop it at sample-accurate points.
    if matches!(graph.playback, Playback::Loop { .. }) && !mono.is_empty() {
        audio::append_smpl_loop(path, graph.sample_rate, 0, mono.len() as u32 - 1)?;
    }
    Ok(())
}

/// Per-channel buffers for a finished render (mono, or stereoized L/R).
fn channels_for(product: &render::RenderProduct, graph: &SoundDoc) -> Vec<Vec<f32>> {
    if let Some((l, r)) = &product.stereo {
        return vec![l.clone(), r.clone()];
    }
    if matches!(graph.stereo, Stereo::Mono) {
        vec![product.mono.clone()]
    } else {
        let (l, r) = render::stereoize(&product.mono, graph.stereo, graph.sample_rate);
        vec![l, r]
    }
}

/// Write a render in the requested container format. WAV goes through
/// `write_render` (stereo + `smpl` loop chunk); FLAC/OGG encode the channels.
fn write_export(
    path: &std::path::Path,
    product: &render::RenderProduct,
    graph: &SoundDoc,
    format: ExportFormat,
    bits: u16,
    quality: f32,
) -> anyhow::Result<()> {
    match format {
        ExportFormat::Wav => write_render(path, product, graph, bits),
        ExportFormat::Flac | ExportFormat::Ogg => {
            let ch = channels_for(product, graph);
            let refs: Vec<&[f32]> = ch.iter().map(|c| c.as_slice()).collect();
            if format == ExportFormat::Flac {
                audio::write_flac(path, &refs, graph.sample_rate, bits)
            } else {
                audio::write_ogg(path, &refs, graph.sample_rate, quality)
            }
        }
    }
}

/// Build a blank 4-layer SFX scaffold: sub / body / top / transient, each a
/// mixer layer with a band-splitting filter, a one-shot envelope, and a
/// starting gain. Sources are neutral placeholders (sine / noise) the agent
/// replaces. Stamped schema v2 (per-layer RNG streams keep the two noise
/// layers independent) and the current engine.
fn scaffold_layered_doc(name: String, base_freq: f32, seed: u64) -> SoundDoc {
    // One role's graph: mul[ chain[source, filter], env ].
    let role = |source: Node, filter: Node, a: f32, d: f32, r: f32, punch: f32| Node::Mul {
        inputs: vec![
            Node::Chain {
                stages: vec![source, filter],
            },
            Node::Env {
                adsr: Adsr {
                    a,
                    d,
                    s: 0.0,
                    r,
                    punch,
                },
            },
        ],
    };
    let sine = |f: f32| Node::Sine {
        freq: Value::Const(f),
    };
    let noise = || Node::Noise {
        color: NoiseColor::White,
    };
    let track = |id: &str, node: Node, gain: f32| Track {
        id: Some(id.to_string()),
        node,
        pan: 0.0,
        gain,
        at: 0.0,
        mute: false,
        automation: Vec::new(),
    };

    let tracks = vec![
        // Sub: an octave-down sine, lowpassed — the weight you feel.
        track(
            "sub",
            role(
                sine(base_freq * 0.5),
                Node::Lowpass {
                    cutoff: Value::Const(140.0),
                    q: 0.7,
                },
                0.0,
                0.18,
                0.06,
                0.0,
            ),
            0.8,
        ),
        // Body: the identity. PLACEHOLDER sine through a bandpass at the
        // fundamental — swap the sine for the real source (fm/super/saw/…).
        track(
            "body",
            role(
                sine(base_freq),
                Node::Bandpass {
                    cutoff: Value::Const(base_freq),
                    q: 1.0,
                },
                0.004,
                0.22,
                0.1,
                0.0,
            ),
            1.0,
        ),
        // Top: highpassed noise — air / sizzle.
        track(
            "top",
            role(
                noise(),
                Node::Highpass {
                    cutoff: Value::Const(4000.0),
                    q: 0.7,
                },
                0.0,
                0.07,
                0.04,
                0.0,
            ),
            0.35,
        ),
        // Transient: a short, punchy noise click — the "now".
        track(
            "transient",
            role(
                noise(),
                Node::Highpass {
                    cutoff: Value::Const(2000.0),
                    q: 0.7,
                },
                0.0,
                0.012,
                0.006,
                0.7,
            ),
            0.7,
        ),
    ];

    SoundDoc {
        name,
        duration: 0.5,
        sample_rate: 44_100,
        seed,
        version: Some(crate::dsl::SCHEMA_VERSION),
        engine: Some(crate::dsl::ENGINE_VERSION),
        stereo: Stereo::Mono,
        normalize: None,
        playback: Playback::OneShot,
        root: Node::Tracks {
            tracks,
            master: Vec::new(),
        },
    }
}

/// Build a tool result carrying a text summary, the structured record, and the
/// spectrogram + waveform images (so the agent can both read stats and "see"
/// the sound). When `include_graph` is set, the graph is added to the
/// structured output (used by mutate/edit so the agent can refine without a
/// `get_sound` round-trip).
fn sound_result(rec: &Record, include_graph: bool) -> CallToolResult {
    let a = &rec.analysis;
    let summary = format!(
        "{} [{}] • {:.3}s • peak {:.1} (true {:.1}) dBFS • rms {:.1} dBFS • {:.1} LUFS • crest {:.1} dB • centroid {:.0} Hz\n\
         attack {:.0} ms • decay/tail {:.0} ms • onsets {} • silence head {:.0} / tail {:.0} ms\n\
         wav: {}\nimages below: spectrogram (freq×time) + waveform (amplitude×time)",
        rec.name,
        rec.id,
        a.duration_secs,
        a.peak_dbfs,
        a.true_peak_dbfs,
        a.rms_dbfs,
        a.loudness_lufs,
        a.crest_factor_db,
        a.spectral_centroid_hz,
        a.attack_time_ms,
        a.decay_time_ms,
        a.onset_count,
        a.head_silence_ms,
        a.tail_silence_ms,
        rec.wav_path.display(),
    );

    let mut content = vec![Content::text(summary)];
    if !a.layers.is_empty() {
        let rows: Vec<String> = a
            .layers
            .iter()
            .map(|l| {
                if l.mute {
                    format!("{} (muted)", l.id)
                } else {
                    format!(
                        "{} {:.0}% • peak {:.1} / rms {:.1} dBFS",
                        l.id, l.energy_pct, l.peak_dbfs, l.rms_dbfs
                    )
                }
            })
            .collect();
        content.push(Content::text(format!(
            "layers (post-fader, pre-master): {}",
            rows.join("  |  ")
        )));
    }
    for img_path in [&a.spectrogram_png_path, &a.waveform_png_path] {
        if let Ok(bytes) = std::fs::read(img_path) {
            let b64 = base64::engine::general_purpose::STANDARD.encode(bytes);
            content.push(Content::image(b64, "image/png"));
        }
    }

    let mut structured = json!({
        "id": rec.id,
        "name": rec.name,
        "wav_path": rec.wav_path.to_string_lossy(),
        "analysis": serde_json::to_value(a).unwrap_or(serde_json::Value::Null),
    });
    if include_graph && let Ok(g) = serde_json::to_value(&rec.graph) {
        structured["graph"] = g;
    }

    let mut result = CallToolResult::success(content);
    result.structured_content = Some(structured);
    result
}

#[tool_handler(router = self.tool_router)]
impl ServerHandler for Sonarium {
    fn get_info(&self) -> ServerInfo {
        let mut info = ServerInfo::default();
        info.capabilities = ServerCapabilities::builder()
            .enable_tools()
            .enable_resources()
            .build();
        // `from_build_env()` reads rmcp's own package name; override with ours.
        let mut imp = Implementation::from_build_env();
        imp.name = env!("CARGO_PKG_NAME").to_string();
        imp.version = env!("CARGO_PKG_VERSION").to_string();
        imp.title = Some("Sonarium".to_string());
        info.server_info = imp;
        info.instructions = Some(INSTRUCTIONS.to_string());
        info
    }

    async fn list_resources(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListResourcesResult, ErrorData> {
        let schema = {
            let mut r = RawResource::new(resources::SCHEMA_URI, "DSL JSON Schema");
            r.description = Some("JSON Schema for the SoundDoc synthesis graph.".into());
            r.mime_type = Some("application/json".into());
            Annotated::new(r, None)
        };
        let cookbook = {
            let mut r = RawResource::new(resources::COOKBOOK_URI, "Cookbook");
            r.description = Some("Example graphs and authoring tips.".into());
            r.mime_type = Some("text/markdown".into());
            Annotated::new(r, None)
        };
        Ok(ListResourcesResult::with_all_items(vec![schema, cookbook]))
    }

    async fn read_resource(
        &self,
        request: ReadResourceRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<ReadResourceResult, ErrorData> {
        let contents = match request.uri.as_str() {
            resources::SCHEMA_URI => ResourceContents::text(resources::schema_json(), &request.uri)
                .with_mime_type("application/json"),
            resources::COOKBOOK_URI => ResourceContents::text(resources::COOKBOOK, &request.uri)
                .with_mime_type("text/markdown"),
            other => {
                return Err(ErrorData::resource_not_found(
                    format!("unknown resource '{other}'"),
                    None,
                ));
            }
        };
        Ok(ReadResourceResult::new(vec![contents]))
    }
}
