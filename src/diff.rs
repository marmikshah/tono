//! `tono diff` — render two documents and report what changed, in numbers.
//!
//! The authoring loop's other half: `tono render` shows what a sound IS;
//! `tono diff A.json B.json` shows what an edit DID — loudness, brightness,
//! envelope, and the sample-domain distance between the two renders.

use tono_core::analysis::{self, Analysis};
use tono_core::dsl::SoundDoc;
use tono_core::render;

/// The full comparison as printable lines.
pub fn diff_report(a: &SoundDoc, b: &SoundDoc) -> String {
    let ra = render::render(a);
    let rb = render::render(b);
    let mut out = String::new();
    if ra == rb {
        out.push_str("sample-identical renders\n");
        return out;
    }
    if a.sample_rate != b.sample_rate {
        out.push_str(&format!(
            "note: sample rates differ ({} vs {} Hz) — stats are computed at each doc's own rate\n",
            a.sample_rate, b.sample_rate
        ));
    }
    let (sa, sb) = (
        analysis::stats(&ra, a.sample_rate),
        analysis::stats(&rb, b.sample_rate),
    );
    out.push_str(&format!(
        "{:<24}{:>12}{:>12}{:>12}\n",
        "metric", "A", "B", "Δ"
    ));
    for (name, va, vb) in metric_rows(&sa, &sb) {
        out.push_str(&format!(
            "{name:<24}{va:>12.3}{vb:>12.3}{:>+12.3}\n",
            vb - va
        ));
    }
    // Sample-domain distance over the overlapping window.
    let n = ra.len().min(rb.len());
    let (mut max_abs, mut sum_sq) = (0.0f32, 0.0f64);
    for i in 0..n {
        let d = (ra[i] - rb[i]).abs();
        max_abs = max_abs.max(d);
        sum_sq += f64::from(d) * f64::from(d);
    }
    let rms = (sum_sq / n as f64).sqrt();
    out.push_str(&format!(
        "\nsample-domain over {n} overlapping samples: max |Δ| {max_abs:.4}, RMS Δ {rms:.4}\n"
    ));
    out
}

/// `(name, a, b)` for every comparable scalar in the analysis.
fn metric_rows(a: &Analysis, b: &Analysis) -> Vec<(&'static str, f32, f32)> {
    vec![
        ("duration_secs", a.duration_secs, b.duration_secs),
        ("loudness_lufs", a.loudness_lufs, b.loudness_lufs),
        ("true_peak_dbfs", a.true_peak_dbfs, b.true_peak_dbfs),
        ("peak_dbfs", a.peak_dbfs, b.peak_dbfs),
        ("rms_dbfs", a.rms_dbfs, b.rms_dbfs),
        ("crest_factor_db", a.crest_factor_db, b.crest_factor_db),
        (
            "centroid_hz",
            a.spectral_centroid_hz,
            b.spectral_centroid_hz,
        ),
        ("flatness", a.spectral_flatness, b.spectral_flatness),
        ("inharmonicity", a.inharmonicity, b.inharmonicity),
        ("attack_time_ms", a.attack_time_ms, b.attack_time_ms),
        (
            "attack_slope_db_per_ms",
            a.attack_slope_db_per_ms,
            b.attack_slope_db_per_ms,
        ),
        ("decay_time_ms", a.decay_time_ms, b.decay_time_ms),
        ("onset_count", a.onset_count as f32, b.onset_count as f32),
        ("head_silence_ms", a.head_silence_ms, b.head_silence_ms),
        ("tail_silence_ms", a.tail_silence_ms, b.tail_silence_ms),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn doc(freq: u32) -> SoundDoc {
        serde_json::from_str(&format!(
            r#"{{ "name":"t", "duration":0.2, "root": {{ "type":"sine", "freq":{freq} }} }}"#
        ))
        .unwrap()
    }

    #[test]
    fn identical_docs_are_sample_identical() {
        assert!(diff_report(&doc(440), &doc(440)).contains("sample-identical"));
    }

    #[test]
    fn a_brighter_doc_reads_as_a_centroid_delta() {
        let report = diff_report(&doc(220), &doc(880));
        assert!(report.contains("centroid_hz"));
        // 880 Hz is brighter: the centroid row shows a positive delta.
        let row = report
            .lines()
            .find(|l| l.starts_with("centroid_hz"))
            .unwrap();
        let delta: f32 = row.split_whitespace().last().unwrap().parse().unwrap();
        assert!(delta > 100.0, "centroid delta: {delta}");
    }

    #[test]
    fn a_softer_doc_reads_as_a_loudness_drop() {
        let loud: SoundDoc = serde_json::from_str(
            r#"{ "name":"t", "duration":0.2, "root": { "type":"sine", "freq":440 } }"#,
        )
        .unwrap();
        let soft: SoundDoc = serde_json::from_str(
            r#"{ "name":"t", "duration":0.2, "root": { "type":"mul", "inputs": [
                { "type":"sine", "freq":440 }, { "type":"env", "s":0.3 } ] } }"#,
        )
        .unwrap();
        let report = diff_report(&loud, &soft);
        let row = report
            .lines()
            .find(|l| l.starts_with("loudness_lufs"))
            .unwrap();
        let delta: f32 = row.split_whitespace().last().unwrap().parse().unwrap();
        assert!(delta < -1.0, "loudness delta: {delta}");
    }
}
