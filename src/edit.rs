//! Surgical, path-addressed edits to a sound graph.
//!
//! Instead of re-submitting the whole [`SoundDoc`] to change one number
//! (`refine_sound`), the agent addresses a node or parameter by a JSON path —
//! `root.inputs[0].freq`, `root.stages[1].cutoff` — and applies small ops:
//! many ordered edits in one render. Edits are applied on the `serde_json`
//! representation, then parsed back to a `SoundDoc` and validated, so an
//! illegal edit is rejected with a readable error rather than producing a
//! broken graph.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value as Json;

use crate::dsl::SoundDoc;

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

/// Apply `ops` to `doc` in order, returning the edited, re-validated graph.
/// An op referencing a missing path, or producing an invalid graph, fails with
/// a message naming the offending op index.
pub fn apply_ops(doc: &SoundDoc, ops: &[EditOp]) -> Result<SoundDoc, String> {
    let mut json = serde_json::to_value(doc).map_err(|e| e.to_string())?;
    for (i, op) in ops.iter().enumerate() {
        apply_one(&mut json, op).map_err(|e| format!("op[{i}]: {e}"))?;
    }
    let edited: SoundDoc =
        serde_json::from_value(json).map_err(|e| format!("edited graph is invalid: {e}"))?;
    edited.validate()?;
    Ok(edited)
}

/// A flattened description of one node in the graph: its path, type, and the
/// immediate (non-child) parameters the agent can address with `set_param`.
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct NodeInfo {
    /// Path to this node (e.g. `root`, `root.inputs[0]`, `root.stages[1]`).
    pub path: String,
    /// Node type (`square`, `lowpass`, `mix`, ...).
    #[serde(rename = "type")]
    pub node_type: String,
    /// Immediate scalar / modulator parameters, keyed by name. Child node arrays
    /// (`inputs` / `stages` / `notes`) are listed as separate `NodeInfo` rows.
    pub params: Json,
}

/// Recognised child-array field names (arrays of nodes / notes).
fn is_child_array(key: &str) -> bool {
    matches!(key, "inputs" | "stages" | "notes")
}

fn walk(json: &Json, path: &str, out: &mut Vec<NodeInfo>) {
    let Some(obj) = json.as_object() else { return };
    if let Some(t) = obj.get("type").and_then(|v| v.as_str()) {
        let mut params = serde_json::Map::new();
        for (k, v) in obj {
            if k == "type" || is_child_array(k) {
                continue;
            }
            params.insert(k.clone(), v.clone());
        }
        out.push(NodeInfo {
            path: path.to_string(),
            node_type: t.to_string(),
            params: Json::Object(params),
        });
    }
    // Recurse into child node arrays regardless (covers nested mix/chain).
    for (k, v) in obj {
        if k == "tracks" {
            // Mixer channels: each element wraps its graph in `node`.
            if let Some(arr) = v.as_array() {
                for (i, ch) in arr.iter().enumerate() {
                    if let Some(node) = ch.get("node") {
                        walk(node, &format!("{path}.tracks[{i}].node"), out);
                    }
                }
            }
            continue;
        }
        if k == "trigger" || k == "node" {
            walk(v, &format!("{path}.{k}"), out);
            continue;
        }
        if !is_child_array(k) {
            continue;
        }
        if let Some(arr) = v.as_array() {
            for (i, child) in arr.iter().enumerate() {
                walk(child, &format!("{path}.{k}[{i}]"), out);
            }
        }
    }
}

/// Produce the addressing map for a graph: one [`NodeInfo`] per node, so the
/// agent can see exactly what paths it can edit before calling `set_param` /
/// `edit_sound`.
pub fn describe(doc: &SoundDoc) -> Vec<NodeInfo> {
    let mut out = Vec::new();
    let Ok(json) = serde_json::to_value(doc) else {
        return out;
    };
    if let Some(root) = json.get("root") {
        walk(root, "root", &mut out);
    }
    out
}

/// Linearly interpolate two same-shaped graphs at `t ∈ [0, 1]`: every numeric
/// parameter is lerped (integers re-rounded), note-name strings are lerped in
/// Hz, and any structural difference is a clear error. `t = 0` ⇒ `a`,
/// `t = 1` ⇒ `b`.
pub fn morph(a: &SoundDoc, b: &SoundDoc, t: f32) -> Result<SoundDoc, String> {
    let ja = serde_json::to_value(a).map_err(|e| e.to_string())?;
    let mut jb = serde_json::to_value(b).map_err(|e| e.to_string())?;
    // Names/version are identity, not parameters — unify before the walk.
    jb["name"] = ja["name"].clone();
    jb["version"] = ja["version"].clone();
    let merged = lerp_json(&ja, &jb, t, "$")?;
    let doc: SoundDoc =
        serde_json::from_value(merged).map_err(|e| format!("morphed graph invalid: {e}"))?;
    doc.validate()?;
    Ok(doc)
}

fn lerp_json(a: &Json, b: &Json, t: f32, path: &str) -> Result<Json, String> {
    match (a, b) {
        (Json::Number(x), Json::Number(y)) => {
            let (fx, fy) = (x.as_f64().unwrap_or(0.0), y.as_f64().unwrap_or(0.0));
            let v = fx + (fy - fx) * t as f64;
            if (x.is_i64() || x.is_u64()) && (y.is_i64() || y.is_u64()) {
                Ok(Json::from(v.round() as i64))
            } else {
                Ok(serde_json::Number::from_f64(v)
                    .map(Json::Number)
                    .unwrap_or_else(|| Json::from(0)))
            }
        }
        (Json::String(x), Json::String(y)) => {
            if x == y {
                Ok(a.clone())
            } else if let (Some(fa), Some(fb)) =
                (crate::dsl::note_to_hz(x), crate::dsl::note_to_hz(y))
            {
                // Two different note names: morph the pitch in Hz.
                let hz = fa + (fb - fa) * t;
                Ok(serde_json::Number::from_f64(hz as f64)
                    .map(Json::Number)
                    .unwrap_or_else(|| Json::from(0)))
            } else {
                Err(format!(
                    "{path}: cannot morph between '{x}' and '{y}' — node types / enum choices must match"
                ))
            }
        }
        (Json::Array(x), Json::Array(y)) => {
            if x.len() != y.len() {
                return Err(format!(
                    "{path}: array lengths differ ({} vs {}) — morph needs identical structure",
                    x.len(),
                    y.len()
                ));
            }
            x.iter()
                .zip(y)
                .enumerate()
                .map(|(i, (xa, xb))| lerp_json(xa, xb, t, &format!("{path}[{i}]")))
                .collect::<Result<Vec<_>, _>>()
                .map(Json::Array)
        }
        (Json::Object(x), Json::Object(y)) => {
            if x.len() != y.len() || x.keys().any(|k| !y.contains_key(k)) {
                return Err(format!(
                    "{path}: object fields differ — morph needs graphs with identical shape"
                ));
            }
            let mut out = serde_json::Map::new();
            for (k, va) in x {
                out.insert(k.clone(), lerp_json(va, &y[k], t, &format!("{path}.{k}"))?);
            }
            Ok(Json::Object(out))
        }
        (Json::Bool(x), Json::Bool(y)) if x == y => Ok(a.clone()),
        (Json::Null, Json::Null) => Ok(Json::Null),
        _ => Err(format!(
            "{path}: structure mismatch — morph interpolates parameters of two same-shaped graphs"
        )),
    }
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
        assert!(err.contains("op[0]"), "{err}");
        assert!(err.contains("no field 'nope'"), "{err}");
        // An edit that parses but breaks validation is also rejected.
        let err = apply_ops(
            &laser(),
            &[op(r#"{ "op": "set", "path": "duration", "value": -1 }"#)],
        )
        .unwrap_err();
        assert!(err.contains("duration"), "{err}");
    }

    #[test]
    fn describe_lists_every_node_with_paths() {
        let infos = describe(&laser());
        let paths: Vec<&str> = infos.iter().map(|i| i.path.as_str()).collect();
        assert_eq!(
            paths,
            vec![
                "root",
                "root.inputs[0]",
                "root.inputs[0].inputs[0]",
                "root.inputs[0].inputs[1]",
                "root.inputs[1]",
            ]
        );
        assert_eq!(infos[2].node_type, "square");
        assert_eq!(infos[2].params["freq"], 880.0);
    }

    #[test]
    fn morph_midpoint_lerps_numbers_and_notes() {
        let a: SoundDoc = serde_json::from_str(
            r#"{ "name": "a", "duration": 0.2, "root": { "type": "sine", "freq": 200 } }"#,
        )
        .unwrap();
        let b: SoundDoc = serde_json::from_str(
            r#"{ "name": "b", "duration": 0.4, "root": { "type": "sine", "freq": 400 } }"#,
        )
        .unwrap();
        let mid = morph(&a, &b, 0.5).unwrap();
        let v = serde_json::to_value(&mid).unwrap();
        assert_eq!(v["root"]["freq"], 300.0);
        assert!((mid.duration - 0.3).abs() < 1e-6);
        // Structural mismatch is a clear error.
        let c: SoundDoc =
            serde_json::from_str(r#"{ "name": "c", "root": { "type": "noise" } }"#).unwrap();
        assert!(morph(&a, &c, 0.5).is_err());
    }
}
