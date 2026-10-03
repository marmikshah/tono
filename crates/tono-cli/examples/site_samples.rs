//! Build the website's real SFX previews from the shared, versioned generator.

use std::{fs, path::PathBuf};

use serde_json::json;
use tono_core::generate::{SfxSpec, SfxTemplate, generate_sfx};

fn main() -> anyhow::Result<()> {
    const USAGE: &str = "Generate the site's WAVs, editable JSON and waveform manifest.\n\nUsage: cargo run --locked --release -p tono --example site_samples -- OUTPUT_DIRECTORY";
    let args: Vec<_> = std::env::args().skip(1).collect();
    let out = match args.as_slice() {
        [help] if help == "--help" || help == "-h" => {
            println!("{USAGE}");
            return Ok(());
        }
        [directory] => PathBuf::from(directory),
        _ => anyhow::bail!(USAGE),
    };
    fs::create_dir_all(&out)?;
    let mut samples = Vec::new();
    for template in SfxTemplate::ALL {
        for seed in 42..46 {
            let spec = SfxSpec::new(template, seed);
            let doc = generate_sfx(&spec)?;
            let audio = tono_core::render::render(&doc);
            let stem = format!("{template}-{seed}");
            let wav = format!("{stem}.wav");
            let source = format!("{stem}.json");
            let mut writer = hound::WavWriter::create(
                out.join(&wav),
                hound::WavSpec {
                    channels: 1,
                    sample_rate: doc.sample_rate,
                    bits_per_sample: 16,
                    sample_format: hound::SampleFormat::Int,
                },
            )?;
            for sample in &audio {
                writer.write_sample((sample.clamp(-1.0, 1.0) * i16::MAX as f32).round() as i16)?;
            }
            writer.finalize()?;
            fs::write(out.join(&source), serde_json::to_string_pretty(&doc)?)?;
            // A compact peak envelope for the web UI, derived from the same
            // samples as the WAV rather than a decorative random waveform.
            let waveform: Vec<f32> = (0..80)
                .map(|bin| {
                    let start = bin * audio.len() / 80;
                    let end = (bin + 1) * audio.len() / 80;
                    audio[start..end]
                        .iter()
                        .map(|value| value.abs())
                        .fold(0.0, f32::max)
                })
                .collect();
            samples.push(json!({
                "template": template,
                "seed": seed,
                "duration": audio.len() as f64 / doc.sample_rate as f64,
                "sampleRate": doc.sample_rate,
                "audio": wav,
                "source": source,
                "waveform": waveform,
                "spec": spec,
            }));
        }
    }
    fs::write(out.join("manifest.json"), serde_json::to_string(&samples)?)?;
    println!("Prepared {} website sound previews", samples.len());
    Ok(())
}
