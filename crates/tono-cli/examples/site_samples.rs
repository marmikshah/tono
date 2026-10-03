//! Export the site's editable SoundDocs and compact waveform manifests.
//!
//! The browser renders these recipes with the same Rust engine through WASM.
//! This build step ships no WAV or OGG files; synthesis here only supplies the
//! waveform envelope shown before a visitor chooses to play a sound.

use std::{
    fs,
    path::{Path, PathBuf},
};

use serde_json::{Value, json};
use tono_core::{
    bgm::{BgmSpec, COMPOSITIONS, VARIANTS as BGM_VARIANTS},
    dsl::SoundDoc,
    generate::{SfxSpec, SfxTemplate, generate_sfx},
    library::{LibrarySpec, RECIPES, VARIANTS},
};

fn main() -> anyhow::Result<()> {
    const USAGE: &str = "Generate the site's editable sound recipes and waveform manifests (no audio files).\n\nUsage: cargo run --locked --release -p tono --example site_samples -- OUTPUT_DIRECTORY";
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
            let stem = format!("{template}-{seed}");
            let exported = export_doc(&out, &stem, &doc)?;
            samples.push(json!({
                "template": template,
                "seed": seed,
                "duration": exported.duration,
                "sampleRate": doc.sample_rate,
                "source": exported.source,
                "waveform": exported.waveform,
                "spec": spec,
            }));
            // Earlier versions generated these exact files. Remove only our
            // own known legacy artifacts, never arbitrary files in the output.
            let legacy = out.join(format!("{stem}.wav"));
            if legacy.is_file() {
                fs::remove_file(legacy)?;
            }
        }
    }
    fs::write(out.join("manifest.json"), serde_json::to_string(&samples)?)?;

    let mut library = Vec::new();
    let mut sound_count = 0;
    for recipe in RECIPES {
        let mut variants = Vec::new();
        for variant in VARIANTS {
            let spec = LibrarySpec::new(recipe.id, variant, 42);
            let doc = tono_core::library::generate(&spec)?;
            let stem = format!("{}-{}", recipe.id, variant.id());
            let exported = export_doc(&out, &stem, &doc)?;
            variants.push(json!({
                "id": variant.id(),
                "label": variant.label(),
                "seed": spec.seed,
                "duration": exported.duration,
                "sampleRate": doc.sample_rate,
                "source": exported.source,
                "waveform": exported.waveform,
                "spec": spec,
            }));
            sound_count += 1;
        }
        library.push(json!({
            "id": recipe.id,
            "title": recipe.title,
            "category": recipe.category,
            "description": recipe.description,
            "tags": recipe.tags,
            "looping": recipe.looping,
            "variants": variants,
        }));
    }
    fs::write(out.join("library.json"), serde_json::to_string(&library)?)?;
    let mut music = Vec::new();
    let mut music_count = 0;
    for composition in COMPOSITIONS {
        let mut variants = Vec::new();
        for variant in BGM_VARIANTS {
            let spec = BgmSpec::new(composition.id, variant, 42);
            let doc = tono_core::bgm::generate(&spec)?;
            let stem = format!("bgm-{}-{}", composition.id, variant.id());
            let exported = export_doc(&out, &stem, &doc)?;
            let score = format!("{stem}.song.json");
            fs::write(
                out.join(&score),
                serde_json::to_string(&tono_core::bgm::score(&spec)?)?,
            )?;
            variants.push(json!({
                "id": variant.id(),
                "label": variant.label(),
                "seed": spec.seed,
                "duration": exported.duration,
                "sampleRate": doc.sample_rate,
                "source": exported.source,
                "waveform": exported.waveform,
                "score": score,
                "spec": spec,
            }));
            music_count += 1;
        }
        music.push(json!({
            "id": composition.id,
            "title": composition.title,
            "category": composition.category,
            "description": composition.description,
            "tags": composition.tags,
            "looping": true,
            "bpm": composition.bpm,
            "key": composition.key,
            "bars": composition.bars,
            "variants": variants,
        }));
    }
    fs::write(out.join("bgm.json"), serde_json::to_string(&music)?)?;
    println!(
        "Prepared {} starter recipes, {sound_count} library sounds, and {music_count} musical loops; no audio files",
        samples.len(),
    );
    Ok(())
}

struct ExportedDoc {
    source: String,
    duration: f64,
    waveform: Value,
}

fn export_doc(out: &Path, stem: &str, doc: &SoundDoc) -> anyhow::Result<ExportedDoc> {
    let source = format!("{stem}.json");
    fs::write(out.join(&source), serde_json::to_string(doc)?)?;
    let audio = tono_core::render::render(doc);
    // A small actual peak envelope. Four decimal places are sufficient for a
    // visual preview and keep the manifest compact; synthesis stays unchanged.
    let waveform: Vec<f64> = (0..80)
        .map(|bin| {
            let start = bin * audio.len() / 80;
            let end = (bin + 1) * audio.len() / 80;
            let peak = audio[start..end]
                .iter()
                .map(|value| value.abs())
                .fold(0.0, f32::max);
            (f64::from(peak) * 10_000.0).round() / 10_000.0
        })
        .collect();
    Ok(ExportedDoc {
        source,
        duration: audio.len() as f64 / doc.sample_rate as f64,
        waveform: json!(waveform),
    })
}
