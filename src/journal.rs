//! Session journal: a replayable record of every mutating tool call.
//!
//! Each successful call that changes state (`author_sound`, `set_param`,
//! `add_to_bank`, ...) appends one line to `session.jsonl` in the working
//! directory: `{ "tool": "...", "args": { ... } }`. Because rendering is
//! deterministic and ids derive from sound names, replaying the journal into a
//! fresh working directory reproduces the entire session — the same sounds,
//! the same banks, byte-identical audio. A journal file is therefore a
//! portable, diffable "project file": share it, version it, or replay it as a
//! starting point for a new session.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value as Json;

/// File name of the journal inside a working directory.
pub const JOURNAL_FILE: &str = "session.jsonl";

/// One recorded tool call: the tool name and its arguments, verbatim.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct Step {
    /// Tool that was called (e.g. `"author_sound"`).
    pub tool: String,
    /// The call's arguments, exactly as received.
    pub args: Json,
}

/// Append-only journal over `<dir>/session.jsonl`. The mutex serialises
/// concurrent appends so lines never interleave.
pub struct Journal {
    path: PathBuf,
    lock: Mutex<()>,
}

impl Journal {
    /// Journal for the working directory `dir` (the file is created on first
    /// append).
    pub fn new(dir: &Path) -> Self {
        Self {
            path: dir.join(JOURNAL_FILE),
            lock: Mutex::new(()),
        }
    }

    /// The journal file's path.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Record one successful mutating tool call. Failures are deliberately not
    /// recorded — a journal must replay cleanly.
    pub fn append(&self, tool: &str, args: &Json) -> anyhow::Result<()> {
        let _guard = self.lock.lock().unwrap();
        let line = serde_json::to_string(&Step {
            tool: tool.to_string(),
            args: args.clone(),
        })?;
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)?;
        writeln!(file, "{line}")?;
        Ok(())
    }

    /// Number of recorded steps (0 if the journal doesn't exist yet).
    pub fn len(&self) -> usize {
        read_steps(&self.path).map(|s| s.len()).unwrap_or(0)
    }

    /// True when nothing has been recorded yet.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// An annotated, shareable session: the documentation-friendly form of a
/// journal. `steps[].note` is for human/agent readers and is ignored on
/// replay; everything else replays exactly like a raw journal.
#[derive(Debug, Deserialize)]
struct Recipe {
    #[allow(dead_code)]
    #[serde(default)]
    name: String,
    #[allow(dead_code)]
    #[serde(default)]
    description: String,
    steps: Vec<RecipeStep>,
}

#[derive(Debug, Deserialize)]
struct RecipeStep {
    tool: String,
    args: Json,
    #[allow(dead_code)]
    #[serde(default)]
    note: String,
}

/// Read every step from a session file: either the raw journal (JSON lines)
/// or an annotated recipe (`{ name, description, steps: [{tool, args, note}] }`).
/// A malformed line is an error, not a skip: a journal that cannot be read in
/// full cannot promise a faithful replay.
pub fn read_steps(path: &Path) -> anyhow::Result<Vec<Step>> {
    let text = std::fs::read_to_string(path)
        .map_err(|e| anyhow::anyhow!("cannot read session file {}: {e}", path.display()))?;
    // Annotated recipe: one JSON object with a `steps` array.
    if text.trim_start().starts_with('{') && text.contains("\"steps\"") {
        let recipe: Recipe = serde_json::from_str(&text)
            .map_err(|e| anyhow::anyhow!("recipe file {}: {e}", path.display()))?;
        return Ok(recipe
            .steps
            .into_iter()
            .map(|s| Step {
                tool: s.tool,
                args: s.args,
            })
            .collect());
    }
    text.lines()
        .filter(|l| !l.trim().is_empty())
        .enumerate()
        .map(|(i, line)| {
            serde_json::from_str(line)
                .map_err(|e| anyhow::anyhow!("session file line {}: {e}", i + 1))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir()
            .join("sonarium_journal_test")
            .join(format!("{tag}_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn append_then_read_roundtrips_in_order() {
        let dir = tmp_dir("roundtrip");
        let j = Journal::new(&dir);
        assert!(j.is_empty());
        j.append(
            "author_sound",
            &serde_json::json!({ "graph": { "name": "beep" } }),
        )
        .unwrap();
        j.append(
            "set_param",
            &serde_json::json!({ "id": "beep", "path": "root.freq", "value": 220 }),
        )
        .unwrap();
        let steps = read_steps(j.path()).unwrap();
        assert_eq!(j.len(), 2);
        assert_eq!(steps[0].tool, "author_sound");
        assert_eq!(steps[1].args["value"], 220);
    }

    #[test]
    fn malformed_line_is_an_error_not_a_skip() {
        let dir = tmp_dir("malformed");
        let j = Journal::new(&dir);
        j.append("author_sound", &serde_json::json!({})).unwrap();
        std::fs::write(
            j.path(),
            format!(
                "{}\nnot json\n",
                std::fs::read_to_string(j.path()).unwrap().trim()
            ),
        )
        .unwrap();
        let err = read_steps(j.path()).unwrap_err().to_string();
        assert!(err.contains("line 2"), "{err}");
    }

    #[test]
    fn annotated_recipes_read_like_journals() {
        let dir = tmp_dir("recipe");
        let path = dir.join("recipe.json");
        std::fs::write(
            &path,
            r#"{ "name": "demo", "description": "two steps",
                 "steps": [
                   { "tool": "author_sound", "args": { "graph": { "name": "a" } },
                     "note": "notes are documentation, ignored on replay" },
                   { "tool": "set_param", "args": { "id": "a" } }
                 ] }"#,
        )
        .unwrap();
        let steps = read_steps(&path).unwrap();
        assert_eq!(steps.len(), 2);
        assert_eq!(steps[0].tool, "author_sound");
        assert_eq!(steps[1].args["id"], "a");
    }

    #[test]
    fn missing_file_reads_as_error_but_len_as_zero() {
        let dir = tmp_dir("missing");
        let j = Journal::new(&dir);
        assert_eq!(j.len(), 0);
        assert!(read_steps(j.path()).is_err());
    }
}
