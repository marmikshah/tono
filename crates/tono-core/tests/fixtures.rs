//! Current patch fixture regression.

use tono_core::patch::Patch;

#[test]
fn current_patch_fixture_loads_and_renders() {
    let json = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/parametric-impact.patch.json"
    ))
    .expect("read patch fixture");
    let patch: Patch = serde_json::from_str(&json).expect("patch parses");
    let samples = patch
        .render(&patch.defaults().into_iter().collect())
        .expect("patch renders with defaults");
    assert!(
        samples.iter().any(|&x| x != 0.0),
        "the patch fixture sounds"
    );
}
