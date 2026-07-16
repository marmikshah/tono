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

    tono vary FILE.json [-n COUNT] [--amount 0..1] [--seed N] [-o DIR] [--format wav|flac|ogg]
        Render COUNT deterministic variations of a SoundDoc (default 4,
        amount 0.15) — round-robin takes of a footstep, impact, pickup.
        Writes <name>_v<i>.json plus the render outputs for each.

    tono schema [sounddoc|patch]
        Print the JSON Schema of the document format (for editor
        autocomplete, validation, and agent self-correction).

    tono midi FILE.json [-o FILE.mid]
        Export a SoundDoc's sequences to a Standard MIDI File.

    tono import FILE.mid [-o DOC.json] [--steps-per-beat 4]
        Import a Standard MIDI File as a renderable SoundDoc of seq
        tracks (GM programs map to the built-in voices; channel 10
        becomes the drum kit).

    tono --version | --help

The SoundDoc format and the node vocabulary are documented in docs/cookbook.md
(https://github.com/marmikshah/tono/blob/master/docs/cookbook.md).";

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    match args.get(1).map(String::as_str) {
        Some("render") => render_cmd(&args[2..]),
        Some("vary") => vary_cmd(&args[2..]),
        Some("schema") => schema_cmd(&args[2..]),
        Some("midi") => midi_cmd(&args[2..]),
        Some("import") => import_cmd(&args[2..]),
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
    // validate() is filesystem-free (the core is pure); the loader owns the
    // existence check so a missing SoundFont still fails loud at load time.
    for sf2 in doc.sf2_paths() {
        if !std::path::Path::new(sf2).exists() {
            anyhow::bail!("seq.sf2: no such file '{sf2}'");
        }
    }
    Ok(doc)
}

fn render_cmd(args: &[String]) -> anyhow::Result<()> {
    let cli = Cli::parse(args, &["-o", "--out", "--format"])?;
    let file = cli.input("tono render FILE.json [-o DIR] [--format wav|flac|ogg]")?;
    let out_dir = PathBuf::from(cli.flag(&["-o", "--out"]).unwrap_or("."));
    let format = parse_format(cli.flag(&["--format"]))?;

    let doc = load_doc(file)?;
    fs::create_dir_all(&out_dir)?;
    let stem = if doc.name.is_empty() {
        Path::new(file)
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("sound")
            .to_string()
    } else {
        doc.name.clone()
    };
    render_to_dir(&doc, &stem, &out_dir, format)
}

/// Validate the `--format` flag once for every rendering command.
fn parse_format(flag: Option<&str>) -> anyhow::Result<&str> {
    let format = flag.unwrap_or("wav");
    if !["wav", "flac", "ogg"].contains(&format) {
        anyhow::bail!("--format must be wav, flac, or ogg, got '{format}'");
    }
    Ok(format)
}

/// The full render pipeline for one doc: audio file (+ `smpl` chunk for loop
/// docs), the two feedback images, and the stats JSON — printing each output
/// path. Shared by `render` and `vary`.
fn render_to_dir(doc: &SoundDoc, stem: &str, out_dir: &Path, format: &str) -> anyhow::Result<()> {
    let product = render::render_product(doc);
    let stereo = product
        .stereo
        .as_ref()
        .map(|(l, r)| (l.as_slice(), r.as_slice()));
    let (left, right) = stereo.unwrap_or((&product.mono, &product.mono));

    let audio_path = out_dir.join(format!("{stem}.{format}"));
    match format {
        "flac" => tono::audio::write_flac(&audio_path, &[left, right], doc.sample_rate, 16)?,
        "ogg" => tono::audio::write_ogg(&audio_path, &[left, right], doc.sample_rate, 0.7)?,
        _ => tono::audio::write_wav_stereo(&audio_path, left, right, doc.sample_rate, 16)?,
    }
    // A `loop` doc's WAV carries a `smpl` chunk spanning the whole rendered
    // loop body, so game engines loop at the sample-accurate points.
    if format == "wav"
        && matches!(doc.playback, tono_core::dsl::Playback::Loop { .. })
        && !left.is_empty()
    {
        tono::audio::append_smpl_loop(
            &audio_path,
            doc.sample_rate,
            0,
            (left.len() as u32).saturating_sub(1),
        )?;
    }

    // The feedback images + numeric analysis — the loop's "look at it" half.
    // Level metrics measure the stereo pair when there is one (the export);
    // the images read the mono mid.
    let png = out_dir.join(format!("{stem}.png"));
    let analysis = tono::imaging::analyze_to_disk(&product.mono, stereo, doc.sample_rate, &png)?;
    let stats = out_dir.join(format!("{stem}.stats.json"));
    fs::write(&stats, serde_json::to_string_pretty(&analysis)?)?;

    println!("{}", audio_path.display());
    println!("{}", png.display());
    println!("{}", analysis.waveform_png_path);
    println!("{}", stats.display());
    Ok(())
}

/// `tono vary` — deterministic round-robin variations of one document.
fn vary_cmd(args: &[String]) -> anyhow::Result<()> {
    let usage = "tono vary FILE.json [-n COUNT] [--amount 0..1] [--seed N] [-o DIR] [--format wav|flac|ogg]";
    let cli = Cli::parse(
        args,
        &[
            "-n", "--count", "--amount", "--seed", "-o", "--out", "--format",
        ],
    )?;
    let file = cli.input(usage)?;
    let out_dir = PathBuf::from(cli.flag(&["-o", "--out"]).unwrap_or("."));
    let format = parse_format(cli.flag(&["--format"]))?;
    let count: u32 = match cli.flag(&["-n", "--count"]) {
        Some(v) => v
            .parse()
            .map_err(|_| anyhow::anyhow!("-n must be a positive integer, got '{v}'"))?,
        None => 4,
    };
    if count == 0 || count > 256 {
        anyhow::bail!("-n must be in 1..=256, got {count}");
    }
    let amount: f32 = match cli.flag(&["--amount"]) {
        Some(v) => v
            .parse()
            .map_err(|_| anyhow::anyhow!("--amount must be a number, got '{v}'"))?,
        None => 0.15,
    };
    if !(0.0..=1.0).contains(&amount) {
        anyhow::bail!("--amount must be in 0..=1, got {amount}");
    }
    let seed: u64 = match cli.flag(&["--seed"]) {
        Some(v) => v
            .parse()
            .map_err(|_| anyhow::anyhow!("--seed must be an integer, got '{v}'"))?,
        None => 0,
    };

    let doc = load_doc(file)?;
    fs::create_dir_all(&out_dir)?;
    let base = if doc.name.is_empty() {
        "sound"
    } else {
        &doc.name
    };

    for i in 1..=count {
        let mut variant = tono_core::vary::mutate(&doc, amount, seed.wrapping_add(i as u64));
        let stem = format!("{base}_v{i}");
        variant.name = stem.clone();
        // mutate() promises a valid doc, but a variant that slipped a bound
        // must fail loud, not render garbage.
        variant
            .validate()
            .map_err(|e| anyhow::anyhow!("variant {i}: {e}"))?;
        let json_path = out_dir.join(format!("{stem}.json"));
        fs::write(&json_path, serde_json::to_string_pretty(&variant)?)?;
        println!("{}", json_path.display());
        render_to_dir(&variant, &stem, &out_dir, format)?;
    }
    Ok(())
}

/// `tono schema` — the machine-readable contract of the document formats.
fn schema_cmd(args: &[String]) -> anyhow::Result<()> {
    let cli = Cli::parse(args, &[])?;
    let target = cli
        .positionals
        .first()
        .map(String::as_str)
        .unwrap_or("sounddoc");
    let schema = match target {
        "sounddoc" => schemars::schema_for!(SoundDoc),
        "patch" => schemars::schema_for!(tono_core::patch::Patch),
        other => anyhow::bail!("unknown schema '{other}' — expected sounddoc or patch"),
    };
    println!("{}", serde_json::to_string_pretty(&schema)?);
    Ok(())
}

fn midi_cmd(args: &[String]) -> anyhow::Result<()> {
    let cli = Cli::parse(args, &["-o", "--out"])?;
    let file = cli.input("tono midi FILE.json [-o FILE.mid]")?;
    let out = PathBuf::from(cli.flag(&["-o", "--out"]).unwrap_or("out.mid"));
    let doc = load_doc(file)?;
    let summary = tono::midi::export_midi(&doc, &out)?;
    println!(
        "{} — {} notes across {} tracks",
        out.display(),
        summary.notes,
        summary.tracks
    );
    Ok(())
}

/// `tono import` — a Standard MIDI File becomes a renderable SoundDoc.
fn import_cmd(args: &[String]) -> anyhow::Result<()> {
    let usage = "tono import FILE.mid [-o DOC.json] [--steps-per-beat 4]";
    let cli = Cli::parse(args, &["-o", "--out", "--steps-per-beat"])?;
    let file = cli.input(usage)?;
    let spb: u32 = match cli.flag(&["--steps-per-beat"]) {
        Some(v) => v.parse().map_err(|_| {
            anyhow::anyhow!("--steps-per-beat must be a positive integer, got '{v}'")
        })?,
        None => 4,
    };
    if spb == 0 || spb > 64 {
        anyhow::bail!("--steps-per-beat must be in 1..=64, got {spb}");
    }
    let out = match cli.flag(&["-o", "--out"]) {
        Some(o) => PathBuf::from(o),
        None => Path::new(file).with_extension("json"),
    };
    let (doc, summary) = tono::midi::import_midi(Path::new(file), spb)?;
    fs::write(&out, serde_json::to_string_pretty(&doc)?)?;
    println!(
        "{} — {} notes across {} tracks at {:.1} bpm",
        out.display(),
        summary.notes,
        summary.tracks,
        summary.bpm
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(a: &[&str]) -> Vec<String> {
        a.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn flags_consume_their_values() {
        // "-o out" must never leave "out" behind as a positional — a real
        // bug class this parser was rewritten to kill.
        let cli = Cli::parse(&args(&["-o", "out", "doc.json"]), &["-o", "--out"]).unwrap();
        assert_eq!(cli.flag(&["-o", "--out"]), Some("out"));
        assert_eq!(cli.positionals, vec!["doc.json"]);
    }

    #[test]
    fn flag_aliases_resolve_in_order() {
        let cli = Cli::parse(&args(&["--out", "d", "f.json"]), &["-o", "--out"]).unwrap();
        assert_eq!(cli.flag(&["-o", "--out"]), Some("d"), "long alias found");
        assert_eq!(cli.flag(&["--missing"]), None, "absent flag is None");
    }

    #[test]
    fn unknown_option_is_a_loud_error() {
        let err = Cli::parse(&args(&["--bogus", "x", "f.json"]), &["-o"])
            .err()
            .unwrap();
        assert!(err.to_string().contains("unknown option '--bogus'"));
    }

    #[test]
    fn flag_missing_its_value_is_a_loud_error() {
        let err = Cli::parse(&args(&["f.json", "-o"]), &["-o"]).err().unwrap();
        assert!(err.to_string().contains("option '-o' needs a value"));
    }

    #[test]
    fn input_wants_exactly_one_positional() {
        let one = Cli::parse(&args(&["f.json"]), &[]).unwrap();
        assert_eq!(one.input("usage").unwrap(), "f.json");

        let none = Cli::parse(&args(&[]), &[]).unwrap();
        assert!(
            none.input("the-usage")
                .err()
                .unwrap()
                .to_string()
                .contains("the-usage")
        );

        let extra = Cli::parse(&args(&["a.json", "b.json"]), &[]).unwrap();
        let msg = extra.input("usage").err().unwrap().to_string();
        assert!(
            msg.contains("unexpected argument 'b.json'"),
            "names the offender: {msg}"
        );
    }

    #[test]
    fn flag_order_does_not_matter() {
        let before = Cli::parse(&args(&["--format", "ogg", "f.json"]), &["--format"]).unwrap();
        let after = Cli::parse(&args(&["f.json", "--format", "ogg"]), &["--format"]).unwrap();
        assert_eq!(before.flag(&["--format"]), after.flag(&["--format"]));
        assert_eq!(before.positionals, after.positionals);
    }
}
