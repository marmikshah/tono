//! `tono match` — target-driven sound design: score a candidate SoundDoc
//! against a reference WAV, in the same numbers the analyzer reports.
//!
//! Bring a sound you like; get told how far off you are and where. The report
//! lists each comparable metric (reference vs candidate), the worst offenders
//! first, plus an overall distance score (0 = identical at these tolerances).

use std::path::Path;

use anyhow::{Context, Result};
use tono_core::analysis::{self, Analysis};
use tono_core::dsl::SoundDoc;
use tono_core::render;

/// Decode a WAV to mono f32 in [-1, 1] plus its sample rate.
pub fn read_wav_mono(path: &Path) -> Result<(Vec<f32>, u32)> {
    let mut reader =
        hound::WavReader::open(path).with_context(|| format!("read WAV {}", path.display()))?;
    let spec = reader.spec();
    let channels = (spec.channels as usize).max(1);
    // hound's duration() is already per-channel (frames), so no division.
    let mut mono = vec![0.0f32; reader.duration() as usize];
    match spec.sample_format {
        hound::SampleFormat::Float => {
            for (i, s) in reader.samples::<f32>().enumerate() {
                mono[i / channels] += s? / channels as f32;
            }
        }
        hound::SampleFormat::Int => {
            let scale = ((1i64 << (spec.bits_per_sample.max(1) - 1)) - 1) as f32;
            for (i, s) in reader.samples::<i32>().enumerate() {
                mono[i / channels] += (s? as f32 / scale) / channels as f32;
            }
        }
    }
    Ok((mono, spec.sample_rate))
}

/// One scored metric: the reference and candidate values, the delta in its
/// natural unit, and the tolerance it is normalized against.
struct Row {
    name: &'static str,
    reference: f32,
    candidate: f32,
    delta: f32,
    tolerance: f32,
}

fn rows(reference: &Analysis, candidate: &Analysis) -> Vec<Row> {
    let mut v = Vec::new();
    let mut row = |name: &'static str, reference: f32, candidate: f32, tolerance: f32| {
        v.push(Row {
            name,
            reference,
            candidate,
            delta: candidate - reference,
            tolerance,
        });
    };
    row(
        "duration_secs",
        reference.duration_secs,
        candidate.duration_secs,
        0.1,
    );
    row(
        "loudness_lufs",
        reference.loudness_lufs,
        candidate.loudness_lufs,
        1.0,
    );
    row(
        "true_peak_dbfs",
        reference.true_peak_dbfs,
        candidate.true_peak_dbfs,
        1.0,
    );
    row(
        "crest_factor_db",
        reference.crest_factor_db,
        candidate.crest_factor_db,
        1.0,
    );
    row(
        "flatness",
        reference.spectral_flatness,
        candidate.spectral_flatness,
        0.05,
    );
    row(
        "inharmonicity",
        reference.inharmonicity,
        candidate.inharmonicity,
        0.05,
    );
    row(
        "attack_time_ms",
        reference.attack_time_ms,
        candidate.attack_time_ms,
        5.0,
    );
    row(
        "decay_time_ms",
        reference.decay_time_ms,
        candidate.decay_time_ms,
        50.0,
    );
    // Brightness compares in octaves (log ratio), not raw Hz.
    let oct = |a: f32, b: f32| (b / a).log2();
    v.push(Row {
        name: "brightness_octaves",
        reference: 0.0,
        candidate: oct(
            reference.spectral_centroid_hz.max(1.0),
            candidate.spectral_centroid_hz.max(1.0),
        ),
        delta: oct(
            reference.spectral_centroid_hz.max(1.0),
            candidate.spectral_centroid_hz.max(1.0),
        ),
        tolerance: 0.5,
    });
    v
}

/// Score `candidate` against a decoded reference (`(mono, sample_rate)`).
/// Lower is closer; 0 means identical within the metric tolerances.
pub fn match_report(reference: &Path, candidate: &SoundDoc) -> Result<String> {
    let (mono, sr) = read_wav_mono(reference)?;
    let ref_stats = analysis::stats(&mono, sr);
    let rendered = render::render(candidate);
    let cand_stats = analysis::stats(&rendered, candidate.sample_rate);

    let rows = rows(&ref_stats, &cand_stats);
    let score = (rows
        .iter()
        .map(|r| {
            let s = r.delta.abs() / r.tolerance;
            s * s
        })
        .sum::<f32>()
        / rows.len() as f32)
        .sqrt();
    let mut worst: Vec<&Row> = rows.iter().collect();
    worst.sort_by(|a, b| (b.delta.abs() / b.tolerance).total_cmp(&(a.delta.abs() / a.tolerance)));

    let mut out = format!(
        "{:<22}{:>12}{:>12}{:>12}\n",
        "metric", "reference", "candidate", "Δ"
    );
    for r in &rows {
        out.push_str(&format!(
            "{:<22}{:>12.3}{:>12.3}{:>+12.3}\n",
            r.name,
            if r.name == "brightness_octaves" {
                ref_stats.spectral_centroid_hz
            } else {
                r.reference
            },
            if r.name == "brightness_octaves" {
                cand_stats.spectral_centroid_hz
            } else {
                r.candidate
            },
            r.delta
        ));
    }
    out.push_str(&format!(
        "\nmatch score: {score:.2}  (RMS distance in tolerance units — 0 is identical)\n"
    ));
    if worst[0].delta.abs() / worst[0].tolerance < 1.0 {
        out.push_str("verdict: close match\n");
    } else {
        out.push_str("worst offenders:\n");
        for r in worst.iter().take(3) {
            out.push_str(&format!(
                "  {:<22} off by {:+.2} ({}× tolerance)\n",
                r.name,
                r.delta,
                (r.delta.abs() / r.tolerance).round() as u32
            ));
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sine_doc(freq: u32) -> SoundDoc {
        serde_json::from_str(&format!(
            r#"{{ "name":"t", "duration":0.3, "root": {{ "type":"sine", "freq":{freq} }} }}"#
        ))
        .unwrap()
    }

    /// An enveloped tone: the decay metric's peak index is stable for this
    /// (on a flat envelope it's a coin flip — the metric, not a bug).
    fn pluck_doc() -> SoundDoc {
        serde_json::from_str(
            r#"{ "name":"t", "duration":0.3, "root": { "type":"mul", "inputs": [
                { "type":"sine", "freq":440 },
                { "type":"env", "a":0.005, "d":0.1, "s":0.0, "r":0.05 } ] } }"#,
        )
        .unwrap()
    }

    fn wav_of(doc: &SoundDoc, name: &str) -> std::path::PathBuf {
        let (l, r) = tono_core::player::render_stereo(doc);
        let dir = std::env::temp_dir().join("tono-target-test");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(name);
        crate::audio::write_wav_stereo(&path, &l, &r, doc.sample_rate, 16).unwrap();
        path
    }

    #[test]
    fn a_doc_matches_its_own_bounce() {
        let doc = pluck_doc();
        let wav = wav_of(&doc, "self.wav");
        let report = match_report(&wav, &doc).unwrap();
        assert!(report.contains("close match"), "{report}");
    }

    #[test]
    fn a_brighter_candidate_is_called_out() {
        let reference = sine_doc(220);
        let candidate = sine_doc(880);
        let wav = wav_of(&reference, "ref.wav");
        let report = match_report(&wav, &candidate).unwrap();
        assert!(report.contains("brightness_octaves"), "{report}");
        assert!(report.contains("worst offenders"), "{report}");
    }

    #[test]
    fn read_wav_mono_decodes_what_we_wrote() {
        let doc = sine_doc(440);
        let wav = wav_of(&doc, "roundtrip.wav");
        let (mono, sr) = read_wav_mono(&wav).unwrap();
        assert_eq!(sr, doc.sample_rate);
        let expected = (doc.duration * doc.sample_rate as f32).round() as usize;
        assert!((mono.len() as i64 - expected as i64).abs() <= 1);
        assert!(mono.iter().all(|x| x.is_finite() && x.abs() <= 1.0));
    }
}
