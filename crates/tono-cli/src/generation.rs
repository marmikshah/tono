//! Discovery and batch export for the core's game SFX starters.

use std::{fs, path::PathBuf};

use clap::Args;
use serde::Serialize;
use tono_core::generate::{SfxSpec, SfxTemplate, generate_sfx};

use super::{Format, format_parser, render_to_dir, unit_interval};

/// `tono generate`: the starter, its seed, the spec's controls, the batch.
#[derive(Args)]
pub(super) struct GenerateArgs {
    /// The starter to generate from (see `tono templates`)
    #[arg(value_name = "TEMPLATE")]
    template: SfxTemplate,

    /// Seed of the first candidate; each later one advances it by one
    #[arg(long, value_name = "N", default_value_t = 0)]
    seed: u64,

    /// How many candidates to generate
    #[arg(short = 'n', long = "count", value_name = "COUNT", default_value_t = 4,
          value_parser = clap::value_parser!(u32).range(1..=32))]
    count: u32,

    /// Spectral tilt [default: 0.5]
    #[arg(long, value_name = "0..1", value_parser = unit_interval)]
    brightness: Option<f32>,

    /// Transient emphasis [default: 0.5]
    #[arg(long, value_name = "0..1", value_parser = unit_interval)]
    punch: Option<f32>,

    /// Spread between the candidates [default: 0.15]
    #[arg(long, value_name = "0..1", value_parser = unit_interval)]
    variation: Option<f32>,

    /// Sample rate in Hz [default: 48000]
    #[arg(long, value_name = "HZ")]
    sample_rate: Option<u32>,

    /// Output directory; must be empty [default: target/generated/<template>-<seed>]
    #[arg(short = 'o', long = "out", value_name = "DIR")]
    out: Option<PathBuf>,

    /// Audio file format
    #[arg(long, value_name = "FORMAT", default_value = "wav", value_parser = format_parser())]
    format: Format,
}

#[derive(Serialize)]
struct Manifest {
    manifest_version: u32,
    preview: String,
    candidates: Vec<Candidate>,
}

#[derive(Serialize)]
struct Candidate {
    spec: SfxSpec,
    document_hash: u64,
    duration_secs: f32,
    document: String,
    audio: String,
    spectrogram: String,
    waveform: String,
    stats: String,
}

pub(super) fn templates_cmd() -> anyhow::Result<()> {
    for template in SfxTemplate::ALL {
        println!("{:<12} {}", template.id(), template.description());
    }
    Ok(())
}

pub(super) fn generate_cmd(args: GenerateArgs) -> anyhow::Result<()> {
    let GenerateArgs {
        template,
        seed,
        count,
        brightness,
        punch,
        variation,
        sample_rate,
        out,
        format,
    } = args;
    let count = count as usize;
    // The spec owns the defaults; a flag overrides one control at a time.
    let mut spec = SfxSpec::new(template, seed);
    spec.brightness = brightness.unwrap_or(spec.brightness);
    spec.punch = punch.unwrap_or(spec.punch);
    spec.variation = variation.unwrap_or(spec.variation);
    spec.sample_rate = sample_rate.unwrap_or(spec.sample_rate);
    spec.validate()?;

    let out = out.unwrap_or_else(|| PathBuf::from(format!("target/generated/{template}-{seed}")));
    // All parameters and documents are checked before creating the directory.
    // A batch owns an empty destination so earlier candidates stay intact.
    if out.exists() {
        anyhow::ensure!(
            out.is_dir(),
            "output must be a directory: {}",
            out.display()
        );
        anyhow::ensure!(
            fs::read_dir(&out)?.next().is_none(),
            "output directory is not empty; choose a new directory: {}",
            out.display()
        );
    }
    let mut candidates = Vec::with_capacity(count);
    let mut docs = Vec::with_capacity(count);
    for index in 0..count {
        let mut take = spec.clone();
        take.seed = seed.wrapping_add(index as u64);
        let doc = generate_sfx(&take)?;
        // Export filenames identify takes; the document remains exactly what
        // the saved spec generates, including its canonical content hash.
        let stem = format!("{template}_v{index}");
        candidates.push(Candidate {
            spec: take,
            document_hash: tono_core::program::content_hash(&doc),
            duration_secs: doc.duration,
            document: format!("{stem}.json"),
            audio: format!("{stem}.{}", format.extension()),
            spectrogram: format!("{stem}.png"),
            waveform: format!("{stem}_wave.png"),
            stats: format!("{stem}.stats.json"),
        });
        docs.push((doc, stem));
    }
    fs::create_dir_all(&out)?;
    for ((doc, stem), candidate) in docs.iter().zip(&candidates) {
        let path = out.join(&candidate.document);
        fs::write(&path, serde_json::to_string_pretty(doc)?)?;
        println!("{}", path.display());
        render_to_dir(doc, stem, &out, format)?;
    }
    let preview = out.join("index.html");
    fs::write(&preview, preview_html(template, &candidates))?;
    println!("{}", preview.display());
    let manifest = out.join("manifest.json");
    fs::write(
        &manifest,
        serde_json::to_string_pretty(&Manifest {
            manifest_version: 1,
            preview: "index.html".into(),
            candidates,
        })?,
    )?;
    println!("{}", manifest.display());
    Ok(())
}

fn preview_html(template: SfxTemplate, candidates: &[Candidate]) -> String {
    let mut html = String::from(
        r#"<!doctype html>
<html lang="en"><meta charset="utf-8"><meta name="viewport" content="width=device-width, initial-scale=1">
<title>tono · Game sound candidates</title>
<style>
:root { color-scheme: dark; font: 16px/1.5 system-ui, sans-serif; background: #111820; color: #e8eef3; }
body { max-width: 1000px; margin: auto; padding: 32px 20px; }
h1 { margin-bottom: 4px; } h2 { margin: 0; font-size: 1.1rem; }
p { color: #adc0cd; } a { color: #97e8ca; }
.grid { display: grid; grid-template-columns: repeat(auto-fit, minmax(min(100%, 340px), 1fr)); gap: 20px; }
article { background: #1a2530; border: 1px solid #354555; border-radius: 12px; padding: 20px; }
audio { display: block; width: 100%; margin: 16px 0; }
img { width: 100%; border-radius: 6px; } .links { display: flex; flex-wrap: wrap; gap: 20px; margin-top: 12px; }
</style><body><main>"#,
    );
    html.push_str(&format!(
        "<h1>{template} candidates</h1><p>{}. Listen, compare, and keep a take.</p><div class=\"grid\">",
        template.description()
    ));
    for (index, candidate) in candidates.iter().enumerate() {
        // Identifiers/filenames come only from our enum and numeric indices;
        // this page does not interpolate arbitrary document names or paths.
        html.push_str(&format!(r#"<article>
<h2>Take {take}</h2><p>Seed {seed} · {duration:.2} seconds · {rate} Hz</p>
<audio controls preload="none" src="./{audio}">Download the audio to listen.</audio>
<img src="./{waveform}" alt="Waveform for take {take}" loading="lazy">
<div class="links"><a href="./{audio}" download>Keep audio</a><a href="./{document}" download>Editable source</a></div>
</article>"#,
            take = index + 1,
            seed = candidate.spec.seed,
            duration = candidate.duration_secs,
            rate = candidate.spec.sample_rate,
            audio = candidate.audio,
            waveform = candidate.waveform,
            document = candidate.document,
        ));
    }
    html.push_str(
        "</div><p><a href=\"./manifest.json\">Generation settings</a></p></main></body></html>",
    );
    html
}
