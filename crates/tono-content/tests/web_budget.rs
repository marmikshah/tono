//! Every authored background music loop must fit the browser adapter's limits.

use tono_content::bgm::{BgmSpec, COMPOSITIONS, VARIANTS, generate};
use tono_web::{MAX_DURATION_SECONDS, MAX_INPUT_BYTES, MAX_SAMPLE_RATE, check_graph_budget};

#[test]
fn every_authored_bgm_fits_the_browser_resource_limits() {
    let mut failures = Vec::new();
    let mut max_graph_work = 0;
    let mut max_note_work = 0;
    for composition in COMPOSITIONS {
        for variant in VARIANTS {
            let spec = BgmSpec::new(composition.id, variant, 42);
            let doc = generate(&spec).unwrap();
            assert!(doc.duration <= MAX_DURATION_SECONDS);
            assert!(doc.sample_rate <= MAX_SAMPLE_RATE);
            assert!(serde_json::to_vec(&doc).unwrap().len() <= MAX_INPUT_BYTES);
            match check_graph_budget(&doc) {
                Ok(budget) => {
                    max_graph_work = max_graph_work.max(budget.work);
                    max_note_work = max_note_work.max(budget.note_work);
                }
                Err(error) => {
                    failures.push(format!("{} / {}: {error}", composition.id, variant.id()));
                }
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
    eprintln!("BGM max graph work: {max_graph_work}; max note work: {max_note_work}");
}
