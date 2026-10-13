//! Surgical, path-addressed edits to a sound graph.
//!
//! Instead of re-submitting the whole [`SoundDoc`] to change one number, a
//! caller addresses a node or parameter by a JSON path —
//! `root.inputs[0].freq`, `root.stages[1].cutoff` — and applies small ops:
//! many ordered edits in one render. Edits are applied on the `serde_json`
//! representation, then parsed back to a `SoundDoc` and validated, so an
//! illegal edit is rejected with a readable error rather than producing a
//! broken graph.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value as Json;

use crate::dsl::{SoundDoc, ValidateError};

/// One element of a path: an object key or an array index.
enum Seg {
    Key(String),
    Index(usize),
}

/// Parse a path like `root.inputs[0].freq`, `root.stages[1].cutoff`, or
/// `root.notes[0].pitch`. Both `name[0]` and `name.0` index forms are accepted.
fn parse_path(path: &str) -> Result<Vec<Seg>, String> {
    let mut segs = Vec::new();
    for raw in path.split('.') {
        if raw.is_empty() {
            continue;
        }
        if let Some(br) = raw.find('[') {
            let key = &raw[..br];
            if !key.is_empty() {
                segs.push(Seg::Key(key.to_string()));
            }
            let mut s = &raw[br..];
            while let Some(rest) = s.strip_prefix('[') {
                let end = rest.find(']').ok_or("unclosed '[' in path")?;
                let num = &rest[..end];
                let idx: usize = num
                    .parse()
                    .map_err(|_| format!("bad array index '{num}' in path"))?;
                segs.push(Seg::Index(idx));
                s = &rest[end + 1..];
            }
            if !s.is_empty() {
                return Err(format!("trailing '{s}' after index in path"));
            }
        } else if let Ok(idx) = raw.parse::<usize>() {
            segs.push(Seg::Index(idx));
        } else {
            segs.push(Seg::Key(raw.to_string()));
        }
    }
    Ok(segs)
}

/// Navigate to a mutable reference at `segs`.
fn nav_mut<'a>(root: &'a mut Json, segs: &[Seg]) -> Result<&'a mut Json, String> {
    let mut cur = root;
    for seg in segs {
        cur = match seg {
            Seg::Key(k) => cur
                .get_mut(k)
                .ok_or_else(|| format!("no field '{k}' at this path"))?,
            Seg::Index(i) => cur
                .get_mut(*i)
                .ok_or_else(|| format!("no array index {i} at this path"))?,
        };
    }
    Ok(cur)
}

/// A single edit operation, externally tagged by `op`. (`Serialize` so the
/// session journal can record edit calls verbatim.)
#[non_exhaustive]
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum EditOp {
    /// Set the JSON value at `path`. Works for a numeric parameter
    /// (`root.inputs[0].freq` → `180` or a modulator object), a scalar field
    /// (`duration`, `seed`, `root.stages[0].q`), or an entire node
    /// (`root.inputs[1]` → `{ "type": "sine", "freq": 220 }`).
    Set {
        /// Path to the target value.
        path: String,
        /// The new JSON value (number, modulator object, or whole node).
        value: Json,
    },
    /// Insert a node into the array at `path` (a `chain`'s `stages` or a
    /// `mix`/`mul`'s `inputs`) at `index` (appended if omitted).
    Insert {
        /// Path to the array (e.g. `root.inputs` or `root.stages`).
        path: String,
        /// Insertion index; appends when omitted or past the end.
        #[serde(default)]
        index: Option<usize>,
        /// The node to insert.
        node: Json,
    },
    /// Remove an array element: either the element at `index` within the array
    /// at `path`, or the element a path ending in `[n]` points to.
    Remove {
        /// Path to an array, or to an array element ending in `[n]`.
        path: String,
        /// Index to remove within the array at `path`.
        #[serde(default)]
        index: Option<usize>,
    },
}

fn apply_one(json: &mut Json, op: &EditOp) -> Result<(), String> {
    match op {
        EditOp::Set { path, value } => {
            let segs = parse_path(path)?;
            if segs.is_empty() {
                return Err("set: path must not be empty".into());
            }
            let target = nav_mut(json, &segs)?;
            *target = value.clone();
            Ok(())
        }
        EditOp::Insert { path, index, node } => {
            let segs = parse_path(path)?;
            let target = nav_mut(json, &segs)?;
            let arr = target
                .as_array_mut()
                .ok_or("insert: path does not point to an array (use a chain's `stages` or a mix/mul `inputs`)")?;
            let i = index.unwrap_or(arr.len()).min(arr.len());
            arr.insert(i, node.clone());
            Ok(())
        }
        EditOp::Remove { path, index } => {
            let segs = parse_path(path)?;
            match index {
                Some(i) => {
                    let arr = nav_mut(json, &segs)?
                        .as_array_mut()
                        .ok_or("remove: path does not point to an array")?;
                    if *i >= arr.len() {
                        return Err(format!(
                            "remove: index {i} out of range (len {})",
                            arr.len()
                        ));
                    }
                    arr.remove(*i);
                    Ok(())
                }
                None => match segs.last() {
                    Some(Seg::Index(i)) => {
                        let i = *i;
                        let parent = nav_mut(json, &segs[..segs.len() - 1])?
                            .as_array_mut()
                            .ok_or("remove: parent of the indexed element is not an array")?;
                        if i >= parent.len() {
                            return Err(format!(
                                "remove: index {i} out of range (len {})",
                                parent.len()
                            ));
                        }
                        parent.remove(i);
                        Ok(())
                    }
                    _ => Err("remove: provide `index`, or a path ending in `[n]`".into()),
                },
            }
        }
    }
}

/// Why an edit failed.
#[derive(Debug, Clone, PartialEq)]
pub enum EditError {
    /// Op `index` failed — a missing path, a bad array index, or a value the
    /// node can't take. The reason names the offending path.
    Op {
        /// Index of the failing op in the batch.
        index: usize,
        /// What went wrong, naming the path.
        reason: String,
    },
    /// The edited JSON no longer deserializes as a `SoundDoc`.
    InvalidGraph(String),
    /// The edited document fails validation.
    Invalid(ValidateError),
    /// `morph`: the two graphs are not the same shape (or hold values that
    /// cannot be interpolated) at the named path.
    Morph(String),
}

impl std::fmt::Display for EditError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            EditError::Op { index, reason } => write!(f, "op[{index}]: {reason}"),
            EditError::InvalidGraph(e) => write!(f, "edited graph is invalid: {e}"),
            EditError::Invalid(e) => write!(f, "edited document fails validation: {e}"),
            EditError::Morph(e) => f.write_str(e),
        }
    }
}

impl std::error::Error for EditError {}

impl From<ValidateError> for EditError {
    fn from(e: ValidateError) -> Self {
        EditError::Invalid(e)
    }
}

/// Apply `ops` to `doc` in order, returning the edited, re-validated graph.
/// An op referencing a missing path, or producing an invalid graph, fails
/// with an [`EditError`] naming the offending op index.
pub fn apply_ops(doc: &SoundDoc, ops: &[EditOp]) -> Result<SoundDoc, EditError> {
    let mut json = serde_json::to_value(doc).map_err(|e| EditError::InvalidGraph(e.to_string()))?;
    for (index, op) in ops.iter().enumerate() {
        apply_one(&mut json, op).map_err(|reason| EditError::Op { index, reason })?;
    }
    let edited: SoundDoc =
        serde_json::from_value(json).map_err(|e| EditError::InvalidGraph(e.to_string()))?;
    edited.validate()?;
    Ok(edited)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn laser() -> SoundDoc {
        serde_json::from_str(
            r#"{ "name": "laser", "duration": 0.2, "root": { "type": "mix", "inputs": [
                { "type": "mul", "inputs": [
                    { "type": "square", "freq": 880, "duty": 0.25 },
                    { "type": "env", "d": 0.18 } ] },
                { "type": "noise" }
            ] } }"#,
        )
        .unwrap()
    }

    fn op(json: &str) -> EditOp {
        serde_json::from_str(json).unwrap()
    }

    #[test]
    fn set_changes_a_nested_param() {
        let edited = apply_ops(
            &laser(),
            &[op(
                r#"{ "op": "set", "path": "root.inputs[0].inputs[0].freq", "value": 440 }"#,
            )],
        )
        .unwrap();
        let v = serde_json::to_value(&edited).unwrap();
        assert_eq!(v["root"]["inputs"][0]["inputs"][0]["freq"], 440.0);
    }

    #[test]
    fn insert_and_remove_reshape_arrays() {
        let edited = apply_ops(
            &laser(),
            &[
                op(r#"{ "op": "insert", "path": "root.inputs",
                         "node": { "type": "sine", "freq": 220 } }"#),
                op(r#"{ "op": "remove", "path": "root.inputs[1]" }"#),
            ],
        )
        .unwrap();
        let v = serde_json::to_value(&edited).unwrap();
        let inputs = v["root"]["inputs"].as_array().unwrap();
        assert_eq!(inputs.len(), 2); // noise removed, sine appended
        assert_eq!(inputs[1]["type"], "sine");
    }

    #[test]
    fn bad_edits_fail_with_op_index_and_reason() {
        let err = apply_ops(
            &laser(),
            &[op(
                r#"{ "op": "set", "path": "root.nope.freq", "value": 1 }"#,
            )],
        )
        .unwrap_err();
        assert!(err.to_string().contains("op[0]"), "{err}");
        assert!(err.to_string().contains("no field 'nope'"), "{err}");
        // An edit that parses but breaks validation is also rejected.
        let err = apply_ops(
            &laser(),
            &[op(r#"{ "op": "set", "path": "duration", "value": -1 }"#)],
        )
        .unwrap_err();
        assert!(err.to_string().contains("duration"), "{err}");
    }
}
