//! Parametric patches — the in-engine runtime.
//!
//! A [`Patch`] is a [`SoundDoc`] template plus named parameters, each bound to
//! one or more JSON paths in the graph. Instantiating it with runtime values
//! produces a concrete document the deterministic renderer turns into audio — so
//! a game ships ONE patch and renders endless per-instance variations (an impact
//! that scales with force, a footstep that varies by surface) with **zero baked
//! files**. Pure and deterministic like the rest of the core: the same patch and
//! the same values always render byte-identically, and it compiles native, to
//! WASM, and into a game engine. This is the thing a DAW structurally can't do.

//!
//! ```
//! use std::collections::BTreeMap;
//! use tono_core::patch::Patch;
//!
//! let patch: Patch = serde_json::from_str(r#"{
//!     "doc": { "name": "zap", "duration": 0.2, "engine": 4,
//!              "root": { "type": "sine", "freq": 880 } },
//!     "params": [ { "name": "pitch", "paths": ["root.freq"],
//!                   "min": 100.0, "max": 2000.0, "default": 880.0 } ]
//! }"#).unwrap();
//!
//! // One patch, endless per-instance variations — deterministic each time.
//! let low = patch.render(&BTreeMap::from([("pitch".into(), 220.0)])).unwrap();
//! let high = patch.render(&BTreeMap::from([("pitch".into(), 1760.0)])).unwrap();
//! assert_ne!(low, high);
//! assert_eq!(low, patch.render(&BTreeMap::from([("pitch".into(), 220.0)])).unwrap());
//! ```

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::dsl::SoundDoc;
use crate::edit::{EditError, EditOp, apply_ops};
use crate::render;

/// A named, range-bounded parameter that drives one or more graph paths.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct ParamSpec {
    /// Semantic name the runtime sets (`"size"`, `"hardness"`, `"surface"`).
    pub name: String,
    /// JSON paths this parameter writes (e.g. `root.modes[0].decay`). One value
    /// can drive several paths at once.
    pub paths: Vec<String>,
    /// Lower bound (values are clamped into `[min, max]`).
    pub min: f32,
    /// Upper bound.
    pub max: f32,
    /// Value used when the runtime doesn't provide one.
    pub default: f32,
}

/// A `SoundDoc` template plus its parameters. Ships as JSON; loaded and rendered
/// at runtime with per-instance values.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct Patch {
    /// The template document.
    pub doc: SoundDoc,
    /// The parameters that vary it.
    #[serde(default)]
    pub params: Vec<ParamSpec>,
}

impl From<SoundDoc> for Patch {
    fn from(doc: SoundDoc) -> Self {
        Patch::new(doc)
    }
}

impl Patch {
    /// A patch around `doc` with no parameters yet — add [`ParamSpec`]s to
    /// expose knobs.
    pub fn new(doc: SoundDoc) -> Self {
        Patch {
            doc,
            params: Vec::new(),
        }
    }

    /// Bake the patch into a concrete document with the given parameter values
    /// (missing → default, out-of-range → clamped). Validated like any edit, so
    /// a bad path or value is a clear error, never a corrupt graph.
    pub fn instantiate(&self, values: &BTreeMap<String, f32>) -> Result<SoundDoc, EditError> {
        let mut ops = Vec::new();
        for spec in &self.params {
            let (lo, hi) = (spec.min.min(spec.max), spec.min.max(spec.max));
            // NaN bounds (programmatic only — JSON can't carry NaN) would
            // panic f32::clamp; treat them as unbounded on that side.
            let lo = if lo.is_nan() { f32::NEG_INFINITY } else { lo };
            let hi = if hi.is_nan() { f32::INFINITY } else { hi };
            let raw = values.get(&spec.name).copied().unwrap_or(spec.default);
            // A NaN (runtime value or default — programmatic only) has no
            // in-domain reading: skip the write and leave the template's own
            // value, rather than panic f32::clamp or bake NaN into the graph.
            if raw.is_nan() {
                continue;
            }
            let v = raw.clamp(lo, hi);
            for path in &spec.paths {
                ops.push(EditOp::Set {
                    path: path.clone(),
                    value: serde_json::json!(v),
                });
            }
        }
        apply_ops(&self.doc, &ops)
    }

    /// Instantiate and render to mono samples — the one call a game makes per
    /// SFX instance.
    pub fn render(&self, values: &BTreeMap<String, f32>) -> Result<Vec<f32>, EditError> {
        Ok(render::render(&self.instantiate(values)?))
    }

    /// The parameter defaults as a value map — a starting point to tweak.
    pub fn defaults(&self) -> BTreeMap<String, f32> {
        self.params
            .iter()
            .map(|p| (p.name.clone(), p.default))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sine_patch() -> Patch {
        let doc: SoundDoc = serde_json::from_str(
            r#"{ "name":"tone", "duration":0.2, "root":{"type":"sine","freq":440} }"#,
        )
        .unwrap();
        Patch {
            doc,
            params: vec![ParamSpec {
                name: "pitch".into(),
                paths: vec!["root.freq".into()],
                min: 100.0,
                max: 2000.0,
                default: 440.0,
            }],
        }
    }

    fn freq_of(doc: &SoundDoc) -> serde_json::Value {
        serde_json::to_value(doc).unwrap()["root"]["freq"].clone()
    }

    #[test]
    fn instantiate_writes_the_param_into_the_graph() {
        let p = sine_patch();
        let mut v = BTreeMap::new();
        v.insert("pitch".to_string(), 880.0);
        assert_eq!(
            freq_of(&p.instantiate(&v).unwrap()),
            serde_json::json!(880.0)
        );
    }

    #[test]
    fn missing_uses_default_and_out_of_range_clamps() {
        let p = sine_patch();
        assert_eq!(
            freq_of(&p.instantiate(&BTreeMap::new()).unwrap()),
            serde_json::json!(440.0)
        );
        let mut v = BTreeMap::new();
        v.insert("pitch".to_string(), 99_999.0);
        assert_eq!(
            freq_of(&p.instantiate(&v).unwrap()),
            serde_json::json!(2000.0)
        );
    }

    #[test]
    fn renders_vary_by_value_and_stay_deterministic() {
        let p = sine_patch();
        let val = |hz: f32| BTreeMap::from([("pitch".to_string(), hz)]);
        let bits = |s: &[f32]| s.iter().map(|x| x.to_bits()).collect::<Vec<_>>();
        let lo = p.render(&val(220.0)).unwrap();
        let hi = p.render(&val(880.0)).unwrap();
        assert!(!lo.is_empty() && lo.len() == hi.len());
        assert_ne!(bits(&lo), bits(&hi), "different value → different audio");
        // Same value twice → byte-identical (the runtime determinism guarantee).
        assert_eq!(bits(&lo), bits(&p.render(&val(220.0)).unwrap()));
    }

    #[test]
    fn out_of_range_default_is_clamped_and_nan_never_panics() {
        let doc: SoundDoc = serde_json::from_str(
            r#"{ "name":"tone", "duration":0.2, "root":{"type":"sine","freq":440} }"#,
        )
        .unwrap();
        // An author default outside [min, max] must not be baked verbatim.
        let p = Patch {
            doc: doc.clone(),
            params: vec![ParamSpec {
                name: "pitch".into(),
                paths: vec!["root.freq".into()],
                min: 100.0,
                max: 2000.0,
                default: 99_999.0,
            }],
        };
        assert_eq!(
            freq_of(&p.instantiate(&BTreeMap::new()).unwrap()),
            serde_json::json!(2000.0)
        );
        // NaN bounds / values (programmatic only) must not panic f32::clamp —
        // a NaN value skips the write and leaves the template's own value.
        let p = Patch {
            doc,
            params: vec![ParamSpec {
                name: "pitch".into(),
                paths: vec!["root.freq".into()],
                min: f32::NAN,
                max: f32::NAN,
                default: 440.0,
            }],
        };
        let mut v = BTreeMap::new();
        v.insert("pitch".to_string(), f32::NAN);
        let d = p.instantiate(&v).unwrap();
        assert_eq!(freq_of(&d), serde_json::json!(440.0));
    }

    /// The shipped example patch parses, its paths are valid, and its parameters
    /// audibly change the sound — so the runtime guide's example actually works.
    #[test]
    fn shipped_impact_patch_renders_across_its_range() {
        let patch: Patch = serde_json::from_str(include_str!(
            "../../../docs/examples/parametric-impact.patch.json"
        ))
        .expect("valid patch");
        let d = patch.render(&patch.defaults()).unwrap();
        assert!(
            !d.is_empty() && d.iter().any(|x| *x != 0.0),
            "defaults make sound"
        );

        let bits = |s: &[f32]| s.iter().map(|x| x.to_bits()).collect::<Vec<_>>();
        let small = patch
            .render(&BTreeMap::from([
                ("size".to_string(), 0.15),
                ("hardness".to_string(), 0.8),
            ]))
            .unwrap();
        let large = patch
            .render(&BTreeMap::from([
                ("size".to_string(), 1.4),
                ("hardness".to_string(), 0.8),
            ]))
            .unwrap();
        assert_ne!(bits(&small), bits(&large), "size changes the ring");
    }
}
