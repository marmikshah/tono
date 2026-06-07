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
use crate::dsl::SoundDoc;

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

/// Session store: working directory + id→record map.
pub struct Store {
    dir: PathBuf,
    map: Mutex<HashMap<String, Record>>,
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
}
