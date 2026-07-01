//! tono — a deterministic sound engine on the command line.
//!
//! Render a `SoundDoc` to audio plus the two feedback images and stats, so any
//! tool — or an agent with a shell — can author sound by the loop: write a doc,
//! render it, look at the spectrogram/waveform, refine.

use std::fs;
use std::path::{Path, PathBuf};

use tono_core::dsl::SoundDoc;
use tono_core::render;

const HELP: &str = "tono — a deterministic sound engine.

USAGE:
    tono render FILE.json [-o DIR] [--format wav|flac|ogg]
        Render a SoundDoc into DIR (default: .):
          <name>.wav|flac|ogg   the audio
          <name>.png            spectrogram   (look at this)
          <name>_wave.png       waveform      (and this)
          <name>.stats.json     peak/RMS/LUFS/spectral/transient analysis

    tono midi FILE.json [-o FILE.mid]
        Export a SoundDoc's sequences to a Standard MIDI File.

    tono --version | --help

The SoundDoc format and the node vocabulary are documented in docs/cookbook.md.";

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    match args.get(1).map(String::as_str) {
        Some("render") => render_cmd(&args[2..]),
        Some("midi") => midi_cmd(&args[2..]),
        Some("--version") | Some("-V") => {
            println!("tono {}", env!("CARGO_PKG_VERSION"));
            Ok(())
        }
        _ => {
            println!("{HELP}");
            Ok(())
        }
    }
}

/// The value after `flag` (e.g. `-o DIR`).
fn opt<'a>(args: &'a [String], flag: &str) -> Option<&'a str> {
    args.iter()
        .position(|a| a == flag)
        .and_then(|i| args.get(i + 1))
        .map(String::as_str)
}

/// The first non-flag argument.
fn positional(args: &[String]) -> Option<&str> {
    args.iter()
        .find(|a| !a.starts_with('-'))
        .map(String::as_str)
}

fn load_doc(path: &str) -> anyhow::Result<SoundDoc> {
    let mut doc: SoundDoc = serde_json::from_str(&fs::read_to_string(path)?)?;
    doc.ensure_track_ids();
    doc.validate().map_err(|e| anyhow::anyhow!(e))?;
    Ok(doc)
}

fn render_cmd(args: &[String]) -> anyhow::Result<()> {
    let file = positional(args).ok_or_else(|| {
        anyhow::anyhow!("usage: tono render FILE.json [-o DIR] [--format wav|flac|ogg]")
    })?;
    let out_dir = PathBuf::from(
        opt(args, "-o")
            .or_else(|| opt(args, "--out"))
            .unwrap_or("."),
    );
    let format = opt(args, "--format").unwrap_or("wav");

    let doc = load_doc(file)?;
    fs::create_dir_all(&out_dir)?;

    let product = render::render_product(&doc);
    let stem = if doc.name.is_empty() {
        Path::new(file)
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("sound")
            .to_string()
    } else {
        doc.name.clone()
    };
    let (left, right) = product
        .stereo
        .clone()
        .unwrap_or_else(|| (product.mono.clone(), product.mono.clone()));

    let audio_path = out_dir.join(format!("{stem}.{}", audio_ext(format)));
    match format {
        "flac" => tono::audio::write_flac(
            &audio_path,
            &[left.as_slice(), right.as_slice()],
            doc.sample_rate,
            16,
        )?,
        "ogg" => tono::audio::write_ogg(
            &audio_path,
            &[left.as_slice(), right.as_slice()],
            doc.sample_rate,
            0.7,
        )?,
        _ => tono::audio::write_wav_stereo(&audio_path, &left, &right, doc.sample_rate, 16)?,
    }

    // The feedback images + numeric analysis — the loop's "look at it" half.
    let png = out_dir.join(format!("{stem}.png"));
    let analysis = tono::imaging::analyze_to_disk(&product.mono, doc.sample_rate, &png)?;
    let stats = out_dir.join(format!("{stem}.stats.json"));
    fs::write(&stats, serde_json::to_string_pretty(&analysis)?)?;

    println!("{}", audio_path.display());
    println!("{}", png.display());
    println!("{}", analysis.waveform_png_path);
    println!("{}", stats.display());
    Ok(())
}

fn audio_ext(format: &str) -> &str {
    match format {
        "flac" => "flac",
        "ogg" => "ogg",
        _ => "wav",
    }
}

fn midi_cmd(args: &[String]) -> anyhow::Result<()> {
    let file = positional(args)
        .ok_or_else(|| anyhow::anyhow!("usage: tono midi FILE.json [-o FILE.mid]"))?;
    let out = PathBuf::from(
        opt(args, "-o")
            .or_else(|| opt(args, "--out"))
            .unwrap_or("out.mid"),
    );
    let doc = load_doc(file)?;
    tono::midi::export_midi(&doc, &out)?;
    println!("{}", out.display());
    Ok(())
}
