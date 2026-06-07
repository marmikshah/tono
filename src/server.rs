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
use crate::dsl::{Normalize, Playback, SoundDoc, Stereo};
use crate::dsp::dbfs;
use crate::edit::{self, EditOp, NodeInfo};
use crate::engines::{self, EngineTarget};
use crate::journal::{self, Journal};
use crate::render;
use crate::resources;
use crate::session::{Record, Store, now_secs, slugify};
use crate::vary;

const INSTRUCTIONS: &str = "Sonarium is a sound-engineering MCP server: you compose audio from instruments and effects by authoring a symbolic synthesis graph, and the server renders it deterministically, returning analysis (peak/true-peak/RMS, LUFS, spectral centroid, transients) plus two images — a spectrogram and a waveform — so you iterate by inspection, like a sound designer at a DAW.\n\
Workflow: author_sound with a graph → read the stats, view the images → refine with set_param / edit_sound (surgical, path-addressed; call describe_sound first to see every editable path) or refine_sound (whole-graph replace) → export (wav/flac/ogg) when it matches. undo_sound / redo_sound step a 20-deep per-sound history; sounds persist across restarts under stable slug ids.\n\
Graph vocabulary: root is one node; every node is a mono signal. Sources: square{freq,duty} (duty modulatable ⇒ PWM), triangle{freq}, sawtooth{freq}, sine{freq}, noise{color: white|pink|brown}, fm{freq,ratio,index} (bells / metallic), super{wave,freq,voices,detune_cents} (supersaw), and seq{bpm,steps_per_beat,wave,duty,env,notes} for melodies/basslines/drums — each note has its own pitch (a number, a NOTE NAME like \"C4\"/\"F#3\"/\"midi:60\", or a slide), a length in grid steps, and the shared per-note ADSR; gaps are rests; notes may overlap. Seq waves are square/triangle/sawtooth/sine/noise plus a core INSTRUMENT list: piano (acoustic — velocity brightness, bass rings/treble dies), epiano (Rhodes tine), organ (drawbars, sustains while held), strings (slow-swell ensemble — write notes slightly early), bass (filtered + sub), kit (drums on the General MIDI map — pitch picks the drum: midi:36 kick, 38 snare, 42/46 hats, 41-50 toms, 49 crash, 51 ride), fm (tunable mallets/bells via fm_ratio/fm_index/fm_strike), pluck (Karplus-Strong string via pluck_decay). Layer seqs like DAW tracks inside a mix. Envelope: env{a,d,s,r,punch}. Combinators: mix (sum), mul (source × env), chain (source → processors). Processors: lowpass/highpass/bandpass/notch{cutoff,q}, peak{cutoff,q,gain_db}, lowshelf/highshelf{cutoff,gain_db}, gain, drive{amount,shape}, ringmod, chorus, flanger, phaser, compress, bitcrush, downsample, delay, reverb. Any numeric param may be a modulator: {\"slide\":{from,to,secs,curve}}, {\"lfo\":{shape,rate,depth,center}}, {\"arp\":{steps,rate}}, {\"env\":{a,d,s,r,from,to}} (the key to filter/pitch envelopes).\n\
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
    /// Every node with its editable path, type, and parameters.
    pub nodes: Vec<NodeInfo>,
}

/// Set a single parameter (or node) by path.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct SetParamReq {
    /// Id of the sound to edit.
    pub id: String,
    /// Path to the target, e.g. `root.inputs[0].freq` or `root.stages[1].cutoff`.
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
    /// Ordered edit ops applied in one re-render.
    pub ops: Vec<EditOp>,
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
        let args = serde_json::to_value(&params.0).map_err(|e| e.to_string())?;
        let AuthorReq { mut graph, name } = params.0;
        if let Some(n) = name {
            graph.name = n;
        }
        let id = self.store.unique_id(&graph.name);
        let rec = self.build(id, graph, now_secs())?;
        self.jlog("author_sound", args);
        Ok(sound_result(&rec, false))
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
        let args = serde_json::to_value(&params.0).map_err(|e| e.to_string())?;
        let RefineReq { id, graph } = params.0;
        let existing = self.require(&id)?;
        self.checkpoint(&id, &existing.graph);
        let rec = self.build(id, graph, existing.created_at)?;
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
        let samples = render::render(&graph);
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
            &samples,
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
        description = "List every node in a sound's graph with its editable path (e.g. root.inputs[0].freq, root.stages[1].cutoff) and parameters. Call this before set_param / edit_sound to see what you can change."
    )]
    pub async fn describe_sound(
        &self,
        params: Parameters<IdReq>,
    ) -> Result<Json<DescribeResp>, String> {
        let rec = self.require(&params.0.id)?;
        Ok(Json(DescribeResp {
            id: rec.id,
            name: rec.name,
            duration: rec.graph.duration,
            nodes: edit::describe(&rec.graph),
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
        let SetParamReq { id, path, value } = params.0;
        let rec = self.require(&id)?;
        let graph = edit::apply_ops(&rec.graph, &[EditOp::Set { path, value }])?;
        self.checkpoint(&id, &rec.graph);
        let new = self.build(id, graph, rec.created_at)?;
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
        let EditReq { id, ops } = params.0;
        if ops.is_empty() {
            return Err("ops must be non-empty".into());
        }
        let rec = self.require(&id)?;
        let graph = edit::apply_ops(&rec.graph, &ops)?;
        self.checkpoint(&id, &rec.graph);
        let new = self.build(id, graph, rec.created_at)?;
        self.jlog("edit_sound", args);
        Ok(sound_result(&new, true))
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
        self.checkpoint(&id, &rec.graph);
        let new = self.build(id, graph, rec.created_at)?;
        self.jlog("make_loop", args);
        // Report how clean the resulting loop seam is.
        let looped = render::render(&new.graph);
        let seam = render::loop_seam_db(&looped);
        let mut res = sound_result(&new, true);
        res.content.push(Content::text(format!(
            "loop: {:.3}s body • seam discontinuity {seam:.1} dB (lower = cleaner; redesign the graph or raise crossfade_secs if high)",
            looped.len() as f32 / new.graph.sample_rate as f32,
        )));
        Ok(res)
    }

    /// Step a sound back to its previous graph.
    #[tool(
        name = "undo_sound",
        description = "Revert a sound to its previous graph (every refine/set_param/edit_sound/make_loop pushes a revision; bounded 20-step history). The undone state moves to the redo stack."
    )]
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

    /// Re-apply the most recently undone edit.
    #[tool(
        name = "redo_sound",
        description = "Re-apply the most recently undone edit to a sound."
    )]
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

    /// Report a sound's undo/redo depths.
    #[tool(
        name = "history",
        description = "Report how many undo / redo revisions a sound has."
    )]
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

    /// Create a named sound bank (a pack).
    #[tool(
        name = "create_bank",
        description = "Create a named sound bank (a pack): a first-class group of sounds you can export together with a manifest. Returns the bank with its stable id."
    )]
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

    /// Add or update a sound's membership in a bank.
    #[tool(
        name = "add_to_bank",
        description = "Add (or update) a sound in a bank with an optional category and round-robin group. Returns the updated bank."
    )]
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

    /// List all sound banks.
    #[tool(
        name = "list_banks",
        description = "List all sound banks and their members."
    )]
    pub async fn list_banks(&self) -> Json<BanksResp> {
        Json(BanksResp {
            banks: self.store.list_banks(),
        })
    }

    /// Export a bank's sounds + a manifest to a directory.
    #[tool(
        name = "export_bank",
        description = "Export every sound in a bank into `dest` plus a sounds.json manifest (id, file, category, rr_group, duration, loudness, peak, channels) so a game engine wires the whole pack with no hand-listing. by_category lays sounds into per-category subfolders. Optional target_lufs level-matches the set."
    )]
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

    /// Export every sound in the library + a manifest to a directory.
    #[tool(
        name = "export_all",
        description = "Export every sound in the library into `dest` with a sounds.json manifest. Library-wide pack export. Optional target_lufs level-matches everything."
    )]
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
            "refine_sound" => self.refine_sound(Parameters(de(a)?)).await.map(drop),
            "set_param" => self.set_param(Parameters(de(a)?)).await.map(drop),
            "edit_sound" => self.edit_sound(Parameters(de(a)?)).await.map(drop),
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
            let samples = render::render(&graph);

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
            write_export(&abs, &samples, &graph, format, 16, quality)
                .map_err(|e| format!("write {}: {e}", abs.display()))?;

            let channels = if matches!(graph.stereo, Stereo::Mono) {
                1
            } else {
                2
            };
            entries.push(ManifestEntry {
                id: rec.id.clone(),
                name: rec.name.clone(),
                file: rel,
                category: m.category.clone(),
                rr_group: m.rr_group.clone(),
                looped: matches!(graph.playback, Playback::Loop { .. }),
                duration_ms: (samples.len() as f32 / graph.sample_rate as f32 * 1000.0).round()
                    as u32,
                sample_rate: graph.sample_rate,
                channels,
                lufs: round2(analysis::loudness_lufs(&samples)),
                peak_dbfs: round2(dbfs(peak_abs(&samples))),
                true_peak_dbfs: round2(dbfs(analysis::true_peak(&samples))),
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
        graph.validate()?;
        let samples = render::render(&graph);

        let wav_path = self.store.wav_path(&id);
        write_render(&wav_path, &samples, &graph, 16)
            .map_err(|e| format!("render write failed: {e}"))?;

        let png_path = self.store.png_path(&id);
        let analysis = analysis::analyze(&samples, graph.sample_rate, &png_path)
            .map_err(|e| format!("analysis failed: {e}"))?;

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
        Ok(rec)
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
    for (id, graph, created_at) in store.list_graph_files() {
        if graph.validate().is_err() {
            continue;
        }
        let samples = render::render(&graph);
        let wav_path = store.wav_path(&id);
        if write_render(&wav_path, &samples, &graph, 16).is_err() {
            continue;
        }
        let png_path = store.png_path(&id);
        let Ok(analysis) = analysis::analyze(&samples, graph.sample_rate, &png_path) else {
            continue;
        };
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

/// Write a mono render to disk, applying the graph's stereo treatment when it
/// isn't `Mono` (then the file is interleaved stereo).
fn write_render(
    path: &std::path::Path,
    mono: &[f32],
    graph: &SoundDoc,
    bits: u16,
) -> anyhow::Result<()> {
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
fn channels_for(samples: &[f32], graph: &SoundDoc) -> Vec<Vec<f32>> {
    if matches!(graph.stereo, Stereo::Mono) {
        vec![samples.to_vec()]
    } else {
        let (l, r) = render::stereoize(samples, graph.stereo, graph.sample_rate);
        vec![l, r]
    }
}

/// Write a render in the requested container format. WAV goes through
/// `write_render` (stereo + `smpl` loop chunk); FLAC/OGG encode the channels.
fn write_export(
    path: &std::path::Path,
    samples: &[f32],
    graph: &SoundDoc,
    format: ExportFormat,
    bits: u16,
    quality: f32,
) -> anyhow::Result<()> {
    match format {
        ExportFormat::Wav => write_render(path, samples, graph, bits),
        ExportFormat::Flac | ExportFormat::Ogg => {
            let ch = channels_for(samples, graph);
            let refs: Vec<&[f32]> = ch.iter().map(|c| c.as_slice()).collect();
            if format == ExportFormat::Flac {
                audio::write_flac(path, &refs, graph.sample_rate, bits)
            } else {
                audio::write_ogg(path, &refs, graph.sample_rate, quality)
            }
        }
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
