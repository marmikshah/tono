//! Session store: in-memory index over on-disk artifacts.
//!
//! The graph is the source of truth. Each sound's WAV, spectrogram PNG, and a
//! copy of its graph JSON live in the working directory so renders are
//! reproducible and version-controllable. Sounds are addressed by a stable,
//! slug-based id usable directly as a game asset key.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::analysis::Analysis;
use crate::bank::Bank;
use crate::dsl::SoundDoc;

/// Prefix marking a bank manifest file (`bank_<id>.json`) so it isn't mistaken
/// for a sound graph during reload.
const BANK_PREFIX: &str = "bank_";

/// One stored sound.
#[derive(Debug, Clone)]
pub struct Record {
    pub id: String,
    pub name: String,
    pub graph: SoundDoc,
    pub wav_path: PathBuf,
    pub analysis: Analysis,
    pub created_at: u64,
}

/// Session store: working directory + id→record map + id→bank map.
pub struct Store {
    dir: PathBuf,
    map: Mutex<HashMap<String, Record>>,
    banks: Mutex<HashMap<String, Bank>>,
    counter: AtomicU64,
}

impl Store {
    /// Create a store rooted at `dir`, creating the directory if needed. The
    /// index starts empty; the server repopulates it from disk on startup.
    pub fn new(dir: PathBuf) -> anyhow::Result<Self> {
        std::fs::create_dir_all(&dir)?;
        Ok(Self {
            dir,
            map: Mutex::new(HashMap::new()),
            banks: Mutex::new(HashMap::new()),
            counter: AtomicU64::new(1),
        })
    }

    /// The working directory where artifacts are written.
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// Allocate a fallback id like `sound_0001` (used only when a name yields no
    /// usable slug). Prefer [`Store::unique_id`].
    pub fn next_id(&self) -> String {
        let n = self.counter.fetch_add(1, Ordering::Relaxed);
        format!("sound_{n:04}")
    }

    /// A stable, collision-free slug id derived from `base` (usually the sound's
    /// name): `"Laser Zap!"` → `laser_zap`, with a numeric suffix on collision.
    /// This id is also the on-disk filename stem and the engine asset key.
    pub fn unique_id(&self, base: &str) -> String {
        let slug = slugify(base);
        let slug = if slug.is_empty() {
            self.next_id()
        } else {
            slug
        };
        if self.id_free(&slug) {
            return slug;
        }
        let mut k = 2u32;
        loop {
            let cand = format!("{slug}_{k}");
            if self.id_free(&cand) {
                return cand;
            }
            k += 1;
        }
    }

    /// True if no record and no on-disk graph already claim `id`.
    fn id_free(&self, id: &str) -> bool {
        !self.map.lock().unwrap().contains_key(id) && !self.dir.join(format!("{id}.json")).exists()
    }

    /// Path to a sound's WAV file.
    pub fn wav_path(&self, id: &str) -> PathBuf {
        self.dir.join(format!("{id}.wav"))
    }

    /// Path to a sound's spectrogram PNG.
    pub fn png_path(&self, id: &str) -> PathBuf {
        self.dir.join(format!("{id}.png"))
    }

    /// Insert or replace a record, persisting its graph alongside the audio.
    pub fn put(&self, record: Record) -> anyhow::Result<()> {
        let json_path = self.dir.join(format!("{}.json", record.id));
        let json = serde_json::to_string_pretty(&record.graph)?;
        std::fs::write(json_path, json)?;
        self.map.lock().unwrap().insert(record.id.clone(), record);
        Ok(())
    }

    /// Fetch a record by id.
    pub fn get(&self, id: &str) -> Option<Record> {
        self.map.lock().unwrap().get(id).cloned()
    }

    /// All records, sorted by creation time.
    pub fn list(&self) -> Vec<Record> {
        let mut v: Vec<Record> = self.map.lock().unwrap().values().cloned().collect();
        v.sort_by(|a, b| a.created_at.cmp(&b.created_at).then(a.id.cmp(&b.id)));
        v
    }

    /// Scan the working directory for persisted sound graphs (`<id>.json`,
    /// excluding `bank_*.json`). Returns `(id, graph, created_at)` so the server
    /// can re-render and re-analyze each on startup. Unparseable files are
    /// skipped — which is also what keeps the `.history.json` / `.redo.json`
    /// stacks out: they hold JSON *arrays*, not `SoundDoc` objects.
    pub fn list_graph_files(&self) -> Vec<(String, SoundDoc, u64)> {
        let mut out = Vec::new();
        let Ok(rd) = std::fs::read_dir(&self.dir) else {
            return out;
        };
        for entry in rd.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }
            let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else {
                continue;
            };
            if stem.starts_with(BANK_PREFIX) {
                continue;
            }
            let Ok(text) = std::fs::read_to_string(&path) else {
                continue;
            };
            let Ok(graph) = serde_json::from_str::<SoundDoc>(&text) else {
                continue;
            };
            let created = entry
                .metadata()
                .ok()
                .and_then(|m| m.modified().ok())
                .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                .map(|d| d.as_secs())
                .unwrap_or(0);
            out.push((stem.to_string(), graph, created));
        }
        out
    }

    // ---- Undo / redo history ----
    //
    // Each sound keeps two bounded on-disk stacks of prior graphs:
    // `<id>.history.json` (undo) and `<id>.redo.json`.

    /// Maximum retained revisions per stack.
    const HISTORY_CAP: usize = 20;

    fn history_path(&self, id: &str) -> PathBuf {
        self.dir.join(format!("{id}.history.json"))
    }
    fn redo_path(&self, id: &str) -> PathBuf {
        self.dir.join(format!("{id}.redo.json"))
    }

    fn read_stack(path: &Path) -> Vec<SoundDoc> {
        std::fs::read_to_string(path)
            .ok()
            .and_then(|t| serde_json::from_str(&t).ok())
            .unwrap_or_default()
    }

    fn write_stack(path: &Path, stack: &[SoundDoc]) {
        if stack.is_empty() {
            let _ = std::fs::remove_file(path);
        } else if let Ok(json) = serde_json::to_string(stack) {
            let _ = std::fs::write(path, json);
        }
    }

    fn push_stack(path: &Path, graph: &SoundDoc) {
        let mut stack = Self::read_stack(path);
        stack.push(graph.clone());
        if stack.len() > Self::HISTORY_CAP {
            let drop = stack.len() - Self::HISTORY_CAP;
            stack.drain(..drop);
        }
        Self::write_stack(path, &stack);
    }

    fn pop_stack(path: &Path) -> Option<SoundDoc> {
        let mut stack = Self::read_stack(path);
        let top = stack.pop();
        Self::write_stack(path, &stack);
        top
    }

    /// Push a graph onto the undo stack (bounded to the last 20 revisions).
    pub fn push_history(&self, id: &str, graph: &SoundDoc) {
        Self::push_stack(&self.history_path(id), graph);
    }

    /// Pop the most recent undo revision.
    pub fn pop_history(&self, id: &str) -> Option<SoundDoc> {
        Self::pop_stack(&self.history_path(id))
    }

    /// Push a graph onto the redo stack.
    pub fn push_redo(&self, id: &str, graph: &SoundDoc) {
        Self::push_stack(&self.redo_path(id), graph);
    }

    /// Pop the most recent redo revision.
    pub fn pop_redo(&self, id: &str) -> Option<SoundDoc> {
        Self::pop_stack(&self.redo_path(id))
    }

    /// Clear the redo stack (a fresh edit invalidates redo).
    pub fn clear_redo(&self, id: &str) {
        let _ = std::fs::remove_file(self.redo_path(id));
    }

    /// Depths of the undo / redo stacks.
    pub fn history_depths(&self, id: &str) -> (usize, usize) {
        (
            Self::read_stack(&self.history_path(id)).len(),
            Self::read_stack(&self.redo_path(id)).len(),
        )
    }

    // ---- Banks ----

    /// A stable, collision-free bank id derived from `base`.
    pub fn unique_bank_id(&self, base: &str) -> String {
        let slug = slugify(base);
        let slug = if slug.is_empty() {
            "bank".to_string()
        } else {
            slug
        };
        if self.bank_id_free(&slug) {
            return slug;
        }
        let mut k = 2u32;
        loop {
            let cand = format!("{slug}_{k}");
            if self.bank_id_free(&cand) {
                return cand;
            }
            k += 1;
        }
    }

    fn bank_id_free(&self, id: &str) -> bool {
        !self.banks.lock().unwrap().contains_key(id)
            && !self.dir.join(format!("{BANK_PREFIX}{id}.json")).exists()
    }

    /// Insert or replace a bank, persisting it as `bank_<id>.json`.
    pub fn put_bank(&self, bank: Bank) -> anyhow::Result<()> {
        let path = self.dir.join(format!("{BANK_PREFIX}{}.json", bank.id));
        std::fs::write(path, serde_json::to_string_pretty(&bank)?)?;
        self.banks.lock().unwrap().insert(bank.id.clone(), bank);
        Ok(())
    }

    /// Fetch a bank by id.
    pub fn get_bank(&self, id: &str) -> Option<Bank> {
        self.banks.lock().unwrap().get(id).cloned()
    }

    /// All banks, sorted by id.
    pub fn list_banks(&self) -> Vec<Bank> {
        let mut v: Vec<Bank> = self.banks.lock().unwrap().values().cloned().collect();
        v.sort_by(|a, b| a.id.cmp(&b.id));
        v
    }

    /// Rehydrate the bank index from `bank_*.json` files on disk.
    pub fn load_banks(&self) {
        let Ok(rd) = std::fs::read_dir(&self.dir) else {
            return;
        };
        for entry in rd.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }
            let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else {
                continue;
            };
            if !stem.starts_with(BANK_PREFIX) {
                continue;
            }
            let Ok(text) = std::fs::read_to_string(&path) else {
                continue;
            };
            if let Ok(bank) = serde_json::from_str::<Bank>(&text) {
                self.banks.lock().unwrap().insert(bank.id.clone(), bank);
            }
        }
    }
}

/// Lowercase, collapse non-alphanumerics to single underscores, trim edges.
/// `"Laser Zap! v2"` → `laser_zap_v2`.
pub fn slugify(s: &str) -> String {
    let mut out = String::new();
    let mut pending_sep = false;
    for c in s.chars() {
        if c.is_ascii_alphanumeric() {
            if pending_sep && !out.is_empty() {
                out.push('_');
            }
            pending_sep = false;
            out.push(c.to_ascii_lowercase());
        } else {
            pending_sep = true;
        }
    }
    out
}

/// Current unix time in seconds.
pub fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    pub(super) fn tmp_store(tag: &str) -> Store {
        let dir = std::env::temp_dir()
            .join("sonarium_session_test")
            .join(format!("{tag}_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        Store::new(dir).unwrap()
    }

    pub(super) fn record(store: &Store, id: &str) -> Record {
        let graph: SoundDoc = serde_json::from_str(&format!(
            r#"{{ "name": "{id}", "duration": 0.05, "root": {{ "type": "sine", "freq": 440 }} }}"#
        ))
        .unwrap();
        let samples = crate::render::render(&graph);
        let analysis =
            crate::analysis::analyze(&samples, graph.sample_rate, &store.png_path(id)).unwrap();
        Record {
            id: id.into(),
            name: id.into(),
            graph,
            wav_path: store.wav_path(id),
            analysis,
            created_at: now_secs(),
        }
    }

    #[test]
    fn slugify_collapses_punctuation_and_case() {
        assert_eq!(slugify("Laser Zap! v2"), "laser_zap_v2");
        assert_eq!(slugify("  UI//Click  "), "ui_click");
        assert_eq!(slugify("!!!"), ""); // nothing usable → empty
    }

    #[test]
    fn unique_id_suffixes_on_collision() {
        let store = tmp_store("ids");
        assert_eq!(store.unique_id("Laser Zap"), "laser_zap");
        store.put(record(&store, "laser_zap")).unwrap();
        assert_eq!(store.unique_id("Laser Zap"), "laser_zap_2");
        // A name with no usable slug falls back to the counter.
        assert!(store.unique_id("!!!").starts_with("sound_"));
    }

    #[test]
    fn put_persists_graph_json_and_get_roundtrips() {
        let store = tmp_store("put");
        store.put(record(&store, "beep")).unwrap();
        assert!(store.dir().join("beep.json").exists());
        let rec = store.get("beep").unwrap();
        assert_eq!(rec.name, "beep");
        assert!(store.get("nope").is_none());
        assert_eq!(store.list().len(), 1);
    }

    #[test]
    fn history_is_lifo_capped_and_redo_clears() {
        let store = tmp_store("history");
        let rec = record(&store, "s");
        // Push 25 revisions with distinguishable durations.
        for i in 0..25u32 {
            let mut g = rec.graph.clone();
            g.duration = 0.01 * (i + 1) as f32;
            store.push_history("s", &g);
        }
        let (undo, redo) = store.history_depths("s");
        assert_eq!((undo, redo), (20, 0)); // capped to the last 20
        let top = store.pop_history("s").unwrap();
        assert!((top.duration - 0.25).abs() < 1e-6); // most recent first
        store.push_redo("s", &top);
        assert_eq!(store.history_depths("s"), (19, 1));
        store.clear_redo("s");
        assert_eq!(store.history_depths("s"), (19, 0));
    }

    #[test]
    fn graph_scan_skips_banks_and_history_stacks() {
        let store = tmp_store("scan");
        store.put(record(&store, "beep")).unwrap();
        store.push_history("beep", &record(&store, "beep").graph);
        store
            .put_bank(Bank {
                id: "ui".into(),
                name: "UI".into(),
                members: vec![],
            })
            .unwrap();
        let found = store.list_graph_files();
        // Only the sound graph — not bank_ui.json, not beep.history.json.
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].0, "beep");
        assert!(found[0].2 > 0); // created_at from file mtime
    }

    #[test]
    fn banks_persist_and_reload() {
        let store = tmp_store("banks");
        assert_eq!(store.unique_bank_id("UI Pack"), "ui_pack");
        store
            .put_bank(Bank {
                id: "ui_pack".into(),
                name: "UI Pack".into(),
                members: vec![],
            })
            .unwrap();
        assert_eq!(store.unique_bank_id("UI Pack"), "ui_pack_2");
        // A fresh store over the same dir reloads the bank from disk.
        let store2 = Store::new(store.dir().to_path_buf()).unwrap();
        assert!(store2.get_bank("ui_pack").is_none());
        store2.load_banks();
        assert_eq!(store2.get_bank("ui_pack").unwrap().name, "UI Pack");
        assert_eq!(store2.list_banks().len(), 1);
    }
}
