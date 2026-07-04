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

/// Parsed command arguments: every flag consumes the value after it, so a
/// flag's value is never mistaken for the input file, and anything unexpected
/// is a loud error instead of a silent default.
struct Cli {
    flags: std::collections::BTreeMap<String, String>,
    positionals: Vec<String>,
}

impl Cli {
    fn parse(args: &[String], allowed_flags: &[&str]) -> anyhow::Result<Cli> {
        let mut flags = std::collections::BTreeMap::new();
        let mut positionals = Vec::new();
        let mut it = args.iter();
        while let Some(a) = it.next() {
            if a.starts_with('-') {
                if !allowed_flags.contains(&a.as_str()) {
                    anyhow::bail!("unknown option '{a}'\n\n{HELP}");
                }
                let value = it
                    .next()
                    .ok_or_else(|| anyhow::anyhow!("option '{a}' needs a value"))?;
                flags.insert(a.clone(), value.clone());
            } else {
                positionals.push(a.clone());
            }
        }
        Ok(Cli { flags, positionals })
    }

    fn flag(&self, names: &[&str]) -> Option<&str> {
        names
            .iter()
            .find_map(|n| self.flags.get(*n))
            .map(String::as_str)
    }

    /// The single expected positional (the input file).
    fn input(&self, usage: &str) -> anyhow::Result<&str> {
        match self.positionals.as_slice() {
            [one] => Ok(one),
            [] => anyhow::bail!("usage: {usage}"),
            more => anyhow::bail!("unexpected argument '{}'\nusage: {usage}", more[1]),
        }
    }
}

fn load_doc(path: &str) -> anyhow::Result<SoundDoc> {
    let mut doc: SoundDoc = serde_json::from_str(&fs::read_to_string(path)?)?;
    doc.ensure_track_ids();
    doc.validate().map_err(|e| anyhow::anyhow!(e))?;
    Ok(doc)
}

fn render_cmd(args: &[String]) -> anyhow::Result<()> {
    let cli = Cli::parse(args, &["-o", "--out", "--format"])?;
    let file = cli.input("tono render FILE.json [-o DIR] [--format wav|flac|ogg]")?;
    let out_dir = PathBuf::from(cli.flag(&["-o", "--out"]).unwrap_or("."));
    let format = cli.flag(&["--format"]).unwrap_or("wav");
    if !["wav", "flac", "ogg"].contains(&format) {
        anyhow::bail!("--format must be wav, flac, or ogg, got '{format}'");
    }

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
    // Level metrics measure the stereo pair when there is one (the export);
    // the images read the mono mid.
    let png = out_dir.join(format!("{stem}.png"));
    let stereo = product
        .stereo
        .as_ref()
        .map(|(l, r)| (l.as_slice(), r.as_slice()));
    let analysis = tono::imaging::analyze_to_disk(&product.mono, stereo, doc.sample_rate, &png)?;
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
    let cli = Cli::parse(args, &["-o", "--out"])?;
    let file = cli.input("tono midi FILE.json [-o FILE.mid]")?;
    let out = PathBuf::from(cli.flag(&["-o", "--out"]).unwrap_or("out.mid"));
    let doc = load_doc(file)?;
    tono::midi::export_midi(&doc, &out)?;
    println!("{}", out.display());
    Ok(())
}
