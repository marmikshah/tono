//! MCP resources: the DSL JSON Schema and a cookbook of example graphs, so the
//! agent can read the vocabulary before authoring.

use crate::dsl::SoundDoc;

/// URI of the DSL JSON Schema resource.
pub const SCHEMA_URI: &str = "tono://schema/sounddoc";
/// URI of the example cookbook resource.
pub const COOKBOOK_URI: &str = "tono://cookbook";

/// The `SoundDoc` JSON Schema, pretty-printed.
pub fn schema_json() -> String {
    let schema = schemars::schema_for!(SoundDoc);
    serde_json::to_string_pretty(&schema).unwrap_or_else(|_| "{}".into())
}

/// The cookbook of example graphs and authoring tips. Single-sourced from
/// `docs/cookbook.md` so the repo doc and the MCP resource can never drift.
pub const COOKBOOK: &str = include_str!("../docs/cookbook.md");

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schema_describes_the_sounddoc() {
        let s = schema_json();
        assert!(s.len() > 1000);
        let v: serde_json::Value = serde_json::from_str(&s).unwrap();
        // Spot-check: the root properties and the node tag are described.
        assert!(v["properties"]["root"].is_object());
        assert!(v["properties"]["duration"].is_object());
    }

    #[test]
    fn cookbook_examples_are_valid_graphs() {
        assert!(COOKBOOK.contains("laser_zap"));
        // Every complete SoundDoc example in the cookbook must parse and
        // validate — the cookbook is a contract, not just prose.
        for block in COOKBOOK.split("```json").skip(1) {
            let Some(code) = block.split("```").next() else {
                continue;
            };
            let trimmed = code.trim();
            // Only full documents (start with '{' and contain "root").
            if !trimmed.starts_with('{') || !trimmed.contains("\"root\"") {
                continue;
            }
            let doc: SoundDoc = serde_json::from_str(trimmed)
                .unwrap_or_else(|e| panic!("cookbook example failed to parse: {e}\n{trimmed}"));
            doc.validate()
                .unwrap_or_else(|e| panic!("cookbook example invalid: {e}\n{trimmed}"));
        }
    }
}
