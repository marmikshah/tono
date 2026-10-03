//! tono — a deterministic sound engine on the command line.
//!
//! Render a `SoundDoc` to audio plus the two feedback images and stats, so any
//! command-line tool can author sound by the loop: write a doc,
//! render it, look at the spectrogram/waveform, refine.

use std::fs;
use std::path::{Path, PathBuf};

use tono_core::dsl::SoundDoc;
use tono_core::render;

mod generation;

const HELP: &str = "tono — a deterministic sound engine.

USAGE:
    tono templates
        List the eight game SFX starters.

    tono generate TEMPLATE [--seed N] [-n COUNT] [-o DIR] [--format wav|flac|ogg]
        Generate COUNT candidates (default 4) with editable JSON, audio,
        feedback images, stats, index.html to audition, and manifest.json.
        DIR must be empty;
        default: target/generated/<template>-<seed> (seed defaults to 0).
        --brightness / --punch: 0..1 (default 0.5)
        --variation: 0..1 (default 0.15); --sample-rate: Hz (default 48000)
        COUNT must be 1..32. Seeds advance by one for each candidate.

    tono render FILE.json [-o DIR] [--format wav|flac|ogg] [--stems DIR] [--watch]
        Render a SoundDoc into DIR (default: .):
          <name>.wav|flac|ogg   the audio
          <name>.png            spectrogram   (look at this)
          <name>_wave.png       waveform      (and this)
          <name>.stats.json     peak/RMS/LUFS/spectral/transient analysis
        --stems also writes every track and bus stem (pre-master) as
        stereo WAVs into DIR. --watch re-renders on every save
        (Ctrl-C to stop) — the edit/inspect loop at full speed.

    tono vary FILE.json [-n COUNT] [--amount 0..1] [--seed N] [-o DIR] [--format wav|flac|ogg]
        Render COUNT deterministic variations of a SoundDoc (default 4,
        amount 0.15) — round-robin takes of a footstep, impact, pickup.
        Writes <name>_v<i>.json plus the render outputs for each.

    tono schema [sounddoc|patch]
        Print the JSON Schema of the document format (for editor
        autocomplete and validation).

    tono midi FILE.json [-o FILE.mid] [--song]
        Export a SoundDoc's sequences to a Standard MIDI File.
        --song reads a Song instead (each song track becomes a
        MIDI track, the kit on channel 10).

    tono compile SONG.json [-o FILE] [--sample-rate N] [--inspect]
        Compile a Song into a validated, hashed Program bundle
        (<name>.program.json by default; an existing default path is
        never overwritten). Every problem is reported in one pass — on
        failure each diagnostic prints with its code, path, and fix,
        and the exit code is non-zero. --inspect prints the
        machine-readable summary (hash, version pins, track roster,
        resource estimates, warnings) as JSON and writes nothing.

    tono import FILE.mid [-o DOC.json] [--steps-per-beat 4] [--song]
        Import a Standard MIDI File as a renderable SoundDoc of seq
        tracks (GM programs map to the built-in voices; channel 10
        becomes the drum kit). --song imports to a Song instead:
        notes land directly on the tracks, no patterns.

    tono diff A.json B.json
        Render both documents and report what changed: loudness, peak,
        brightness, envelope metrics, and the sample-domain distance.

    tono match REF.wav DOC.json
        Score a SoundDoc against a reference WAV — how close it is and
        where it misses (brightness, loudness, envelope, duration).

    tono fit REF.wav DOC.json [-o FITTED.json] [--rounds N] [--amount 0..1] [--seed N]
        Hill-climb the doc's parameters toward the reference WAV: a
        deterministic seeded search over vary mutations that keeps every
        improvement, then writes the best doc (default <doc>.fit.json)
        and prints its final match report.

    tono review FILE.json [--archetype KIND]
        Grade a SoundDoc against the ship checklist (and an archetype's
        targets: laser, coin, jump, impact, ui, footstep, powerup,
        ambience, bgm): every finding names the measured value, the
        target, and the fix to try. Exits non-zero on a FAIL grade.

    tono play FILE.json [--secs N]
        Audition a SoundDoc through the speakers (needs the `play`
        feature: cargo install tono --features play).

    tono presets [NAME] [-o DIR] [--format wav|flac|ogg]
        No NAME: list the 16 factory presets. With NAME: render the
        preset's demo riff (a C-major arpeggio through the live
        Instrument engine) — the same outputs as 'tono render'.

    tono catalog [NAME] [-o DIR] [--format wav|flac|ogg]
        No NAME: list the 31 catalog voices by family. With NAME: render
        the voice's demo — a C-major scale resolving to a chord (a
        two-bar groove for the drum kits).

    tono --version | --help

The SoundDoc format and the node vocabulary are documented in the SoundDoc reference
(https://marmikshah.github.io/tono/reference/sounddoc).";

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    match args.get(1).map(String::as_str) {
        Some("templates") => generation::templates_cmd(&args[2..]),
        Some("generate") => generation::generate_cmd(&args[2..]),
        Some("render") => render_cmd(&args[2..]),
        Some("vary") => vary_cmd(&args[2..]),
        Some("schema") => schema_cmd(&args[2..]),
        Some("midi") => midi_cmd(&args[2..]),
        Some("compile") => compile_cmd(&args[2..]),
        Some("import") => import_cmd(&args[2..]),
        Some("diff") => diff_cmd(&args[2..]),
        Some("match") => match_cmd(&args[2..]),
        Some("fit") => fit_cmd(&args[2..]),
        Some("review") => review_cmd(&args[2..]),
        Some("play") => play_cmd(&args[2..]),
        Some("presets") => presets_cmd(&args[2..]),
        Some("catalog") => catalog_cmd(&args[2..]),
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
    bools: std::collections::BTreeSet<String>,
    positionals: Vec<String>,
}

impl Cli {
    fn parse(args: &[String], allowed_flags: &[&str], bool_flags: &[&str]) -> anyhow::Result<Cli> {
        let mut flags = std::collections::BTreeMap::new();
        let mut bools = std::collections::BTreeSet::new();
        let mut positionals = Vec::new();
        let mut it = args.iter();
        while let Some(a) = it.next() {
            if a.starts_with('-') {
                if bool_flags.contains(&a.as_str()) {
                    bools.insert(a.clone());
                    continue;
                }
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
        Ok(Cli {
            flags,
            bools,
            positionals,
        })
    }

    fn flag(&self, names: &[&str]) -> Option<&str> {
        names
            .iter()
            .find_map(|n| self.flags.get(*n))
            .map(String::as_str)
    }

    /// Whether a value-less flag was passed (e.g. `--watch`).
    fn has(&self, name: &str) -> bool {
        self.bools.contains(name)
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

/// A doc name doubles as an output file stem: reject anything that would
/// escape the output directory (separators, parent refs, Windows drive
/// prefixes) instead of failing later with a bare OS error.
fn sanitize_stem(name: &str) -> anyhow::Result<String> {
    if name.is_empty() || name == "." || name == ".." || name.contains(['/', '\\', ':']) {
        anyhow::bail!(
            "doc name '{name}' can't name an output file (no path separators or parent refs)"
        );
    }
    Ok(name.to_string())
}

fn render_cmd(args: &[String]) -> anyhow::Result<()> {
    let cli = Cli::parse(args, &["-o", "--out", "--format", "--stems"], &["--watch"])?;
    let file = cli
        .input("tono render FILE.json [-o DIR] [--format wav|flac|ogg] [--stems DIR] [--watch]")?;
    let out_dir = PathBuf::from(cli.flag(&["-o", "--out"]).unwrap_or("."));
    let stems_dir = cli.flag(&["--stems"]).map(PathBuf::from);
    let format = parse_format(cli.flag(&["--format"]))?;
    let watch = cli.has("--watch");
    fs::create_dir_all(&out_dir)?;
    if let Some(dir) = &stems_dir {
        fs::create_dir_all(dir)?;
    }

    // Render once, then re-render on every save. With --watch, an edit that
    // leaves the doc invalid is reported and watched through, not fatal.
    let mut since: Option<std::time::SystemTime> = None;
    loop {
        let result = load_doc(file).and_then(|doc| {
            let stem = if doc.name.is_empty() {
                Path::new(file)
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or("sound")
                    .to_string()
            } else {
                sanitize_stem(&doc.name)?
            };
            render_to_dir(&doc, &stem, &out_dir, format)?;
            if let Some(dir) = &stems_dir {
                write_stems(&doc, &stem, dir)?;
            }
            Ok(())
        });
        match result {
            Ok(()) => {}
            Err(e) if watch => eprintln!("{e:#}"),
            Err(e) => return Err(e),
        }
        if !watch {
            break;
        }
        since = Some(wait_for_change(
            file,
            since.or_else(|| file_mtime(file).ok()),
        )?);
    }
    Ok(())
}

/// The file's modification time.
fn file_mtime(path: &str) -> std::io::Result<std::time::SystemTime> {
    fs::metadata(path).and_then(|m| m.modified())
}

/// Poll the file's mtime until it changes (250 ms ticks — no dependency).
/// `since` is the mtime of the last render; `None` means the first change
/// from whatever is there now.
fn wait_for_change(
    path: &str,
    since: Option<std::time::SystemTime>,
) -> anyhow::Result<std::time::SystemTime> {
    loop {
        let mtime = file_mtime(path)?;
        match since {
            Some(prev) if mtime == prev => {
                std::thread::sleep(std::time::Duration::from_millis(250))
            }
            _ => return Ok(mtime),
        }
    }
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
/// path. Shared by `render`, `vary`, and `generate`.
fn render_to_dir(doc: &SoundDoc, stem: &str, out_dir: &Path, format: &str) -> anyhow::Result<()> {
    let product = render::render_product(doc);
    let treated = if product.stereo.is_none() && !matches!(doc.stereo, tono_core::dsl::Stereo::Mono)
    {
        Some(render::stereoize(
            &product.mono,
            doc.stereo,
            doc.sample_rate,
        ))
    } else {
        None
    };
    let stereo = product
        .stereo
        .as_ref()
        .or(treated.as_ref())
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

/// `tono render --stems`: write every track and bus stem as a stereo WAV
/// (pre-master-chain — they feed an external mixer). Stem ids carry `bus:`
/// prefixes, sanitized to `bus_` for portable filenames.
fn write_stems(doc: &SoundDoc, stem: &str, dir: &Path) -> anyhow::Result<()> {
    let stems = tono_core::render::render_stems(doc).ok_or_else(|| {
        anyhow::anyhow!("--stems needs a tracks document (this one has a single-graph root)")
    })?;
    for s in &stems {
        let safe_id = s.id.replace(':', "_");
        let safe_id = sanitize_stem(&safe_id)?;
        let path = dir.join(format!("{stem}_{safe_id}.wav"));
        tono::audio::write_wav_stereo(&path, &s.left, &s.right, doc.sample_rate, 16)?;
        println!("{}", path.display());
    }
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
        &[],
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
        "sound".to_string()
    } else {
        sanitize_stem(&doc.name)?
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
    let cli = Cli::parse(args, &[], &[])?;
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
    let cli = Cli::parse(args, &["-o", "--out"], &["--song"])?;
    let file = cli.input("tono midi FILE.json [-o FILE.mid] [--song]")?;
    let out = match cli.flag(&["-o", "--out"]) {
        Some(o) => PathBuf::from(o),
        // A defaulted output must never silently clobber an existing file.
        None if Path::new("out.mid").exists() => {
            anyhow::bail!("out.mid already exists — pass -o to choose a different output")
        }
        None => PathBuf::from("out.mid"),
    };
    // --song switches the input format: a Song lowers through to_doc and its
    // tracks become the MIDI tracks; without it the input is a SoundDoc.
    let summary = if cli.has("--song") {
        let song: tono_core::song::Song = serde_json::from_str(&fs::read_to_string(file)?)
            .map_err(|e| anyhow::anyhow!("parsing {file} as a Song: {e}"))?;
        tono::midi::export_song_midi(&song, &out)?
    } else {
        let doc = load_doc(file)?;
        tono::midi::export_midi(&doc, &out)?
    };
    println!(
        "{} — {} notes across {} tracks",
        out.display(),
        summary.notes,
        summary.tracks
    );
    Ok(())
}

/// `tono compile` — a Song becomes a validated, hashed Program bundle.
fn compile_cmd(args: &[String]) -> anyhow::Result<()> {
    let usage = "tono compile SONG.json [-o FILE] [--sample-rate N] [--inspect]";
    let cli = Cli::parse(args, &["-o", "--out", "--sample-rate"], &["--inspect"])?;
    let file = cli.input(usage)?;
    let sample_rate =
        match cli.flag(&["--sample-rate"]) {
            Some(v) => Some(v.parse::<u32>().map_err(|_| {
                anyhow::anyhow!("--sample-rate must be a positive integer, got '{v}'")
            })?),
            None => None,
        };
    let program = tono::compile::compile_song(file, sample_rate)?;
    for w in &program.warnings() {
        eprintln!("{} {} {}: {}", w.severity, w.code, w.path, w.message);
    }
    if cli.has("--inspect") {
        let inspect = tono::compile::inspect_json(&program);
        println!("{}", serde_json::to_string_pretty(&inspect)?);
        return Ok(());
    }
    let out = match cli.flag(&["-o", "--out"]) {
        Some(o) => PathBuf::from(o),
        None => {
            let stem = if program.doc.name.is_empty() {
                Path::new(file)
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or("song")
                    .to_string()
            } else {
                sanitize_stem(&program.doc.name)?
            };
            let default = PathBuf::from(format!("{stem}.program.json"));
            // A defaulted output must never silently clobber an existing file.
            if default.exists() {
                anyhow::bail!(
                    "{} already exists — pass -o to choose a different output",
                    default.display()
                );
            }
            default
        }
    };
    fs::write(&out, program.to_json())?;
    println!(
        "{} — hash {:#018x}, {:.2}s, {} tracks, {} events",
        out.display(),
        program.hash,
        program.doc.duration,
        program.meta.tracks.len(),
        program.estimates.events,
    );
    Ok(())
}

/// `tono import` — a Standard MIDI File becomes a renderable SoundDoc.
fn import_cmd(args: &[String]) -> anyhow::Result<()> {
    let usage = "tono import FILE.mid [-o DOC.json] [--steps-per-beat 4] [--song]";
    let cli = Cli::parse(args, &["-o", "--out", "--steps-per-beat"], &["--song"])?;
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
        None => {
            // A defaulted output must never silently clobber an existing file
            // (e.g. an authored doc sharing the MIDI file's stem).
            let default = Path::new(file).with_extension("json");
            if default.exists() {
                anyhow::bail!(
                    "{} already exists — pass -o to choose a different output",
                    default.display()
                );
            }
            default
        }
    };
    // --song switches the output format: the tracks become SongTracks with the
    // notes written directly; without it the output is a SoundDoc.
    if cli.has("--song") {
        let song = tono::midi::import_midi_song(Path::new(file), spb)?;
        let notes: usize = song.tracks.iter().map(|t| t.notes.len()).sum();
        fs::write(&out, serde_json::to_string_pretty(&song)?)?;
        println!(
            "{} — {} notes across {} tracks at {:.1} bpm",
            out.display(),
            notes,
            song.tracks.len(),
            song.bpm
        );
    } else {
        let (doc, summary) = tono::midi::import_midi(Path::new(file), spb)?;
        fs::write(&out, serde_json::to_string_pretty(&doc)?)?;
        println!(
            "{} — {} notes across {} tracks at {:.1} bpm",
            out.display(),
            summary.notes,
            summary.tracks,
            summary.bpm
        );
    }
    Ok(())
}

/// `tono diff` — what did the edit actually do? Render both docs and report
/// the changed numbers (and the sample-domain distance).
fn diff_cmd(args: &[String]) -> anyhow::Result<()> {
    let cli = Cli::parse(args, &[], &[])?;
    let [a, b] = match cli.positionals.as_slice() {
        [a, b] => [a, b],
        _ => anyhow::bail!("usage: tono diff A.json B.json"),
    };
    let (da, db) = (load_doc(a)?, load_doc(b)?);
    print!("{}", tono::diff::diff_report(&da, &db));
    Ok(())
}

/// `tono match` — score a candidate doc against a reference WAV.
fn match_cmd(args: &[String]) -> anyhow::Result<()> {
    let cli = Cli::parse(args, &[], &[])?;
    let [reference, candidate] = match cli.positionals.as_slice() {
        [a, b] => [a, b],
        _ => anyhow::bail!("usage: tono match REF.wav DOC.json"),
    };
    let doc = load_doc(candidate)?;
    print!(
        "{}",
        tono::target::match_report(Path::new(reference), &doc)?
    );
    Ok(())
}

/// `tono fit` — hill-climb a doc toward a reference WAV, write the best doc.
fn fit_cmd(args: &[String]) -> anyhow::Result<()> {
    let usage =
        "tono fit REF.wav DOC.json [-o FITTED.json] [--rounds N] [--amount 0..1] [--seed N]";
    let cli = Cli::parse(
        args,
        &["-o", "--out", "--rounds", "--amount", "--seed"],
        &[],
    )?;
    let [reference, candidate] = match cli.positionals.as_slice() {
        [a, b] => [a, b],
        _ => anyhow::bail!("usage: {usage}"),
    };
    let rounds: u32 = match cli.flag(&["--rounds"]) {
        Some(v) => v
            .parse()
            .map_err(|_| anyhow::anyhow!("--rounds must be a positive integer, got '{v}'"))?,
        None => 32,
    };
    if rounds == 0 || rounds > 4096 {
        anyhow::bail!("--rounds must be in 1..=4096, got {rounds}");
    }
    let amount: f32 = match cli.flag(&["--amount"]) {
        Some(v) => v
            .parse()
            .map_err(|_| anyhow::anyhow!("--amount must be a number, got '{v}'"))?,
        None => 0.25,
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
    let out = match cli.flag(&["-o", "--out"]) {
        Some(o) => PathBuf::from(o),
        None => {
            // A defaulted output must never silently clobber an existing file.
            let default = Path::new(candidate).with_extension("fit.json");
            if default.exists() {
                anyhow::bail!(
                    "{} already exists — pass -o to choose a different output",
                    default.display()
                );
            }
            default
        }
    };

    let doc = load_doc(candidate)?;
    let result = tono::fit::fit(Path::new(reference), &doc, rounds, amount, seed)?;
    fs::write(&out, serde_json::to_string_pretty(&result.doc)?)?;
    println!(
        "fit: {:.2} → {:.2} over {} rounds ({} improvements)",
        result.initial, result.score, result.rounds, result.improvements
    );
    println!("{}", out.display());
    // The full metric table for the fitted doc, so the remaining gap is
    // visible without a second command.
    print!(
        "{}",
        tono::target::match_report(Path::new(reference), &result.doc)?
    );
    Ok(())
}

/// `tono review` — grade a doc against the ship checklist (and an archetype).
fn review_cmd(args: &[String]) -> anyhow::Result<()> {
    let usage = "tono review FILE.json [--archetype laser|coin|jump|impact|ui|footstep|powerup|ambience|bgm]";
    let cli = Cli::parse(args, &["--archetype"], &[])?;
    let file = cli.input(usage)?;
    let archetype = cli
        .flag(&["--archetype"])
        .map(tono::review::parse_archetype)
        .transpose()?;
    let doc = load_doc(file)?;
    let (review, report) = tono::review::review_doc(&doc, archetype);
    print!("{report}");
    // A FAIL grade is a ship-blocker — say so with the exit code.
    if review.grade == tono::review::Status::Fail {
        anyhow::bail!("{} FAIL finding(s) — fix before shipping", review.fail);
    }
    Ok(())
}

/// `tono play` — audition a doc through the speakers (feature `play`).
#[cfg(feature = "play")]
fn play_cmd(args: &[String]) -> anyhow::Result<()> {
    let cli = Cli::parse(args, &["--secs"], &[])?;
    let file = cli.input("tono play FILE.json [--secs N]")?;
    let doc = load_doc(file)?;
    let secs: f32 = match cli.flag(&["--secs"]) {
        Some(v) => v
            .parse()
            .map_err(|_| anyhow::anyhow!("--secs must be a number, got '{v}'"))?,
        None => doc.duration + 1.0,
    };
    tono::play::play_doc(&doc, secs)
}

/// Without the `play` feature the subcommand still parses, but says why it
/// can't help — a discoverable error, not "unknown option".
#[cfg(not(feature = "play"))]
fn play_cmd(_args: &[String]) -> anyhow::Result<()> {
    anyhow::bail!(
        "this build has no audio playback — rebuild with `cargo install tono --features play`"
    )
}

/// `tono presets` — list the factory presets, or render one's demo riff.
fn presets_cmd(args: &[String]) -> anyhow::Result<()> {
    let usage = "tono presets [NAME] [-o DIR] [--format wav|flac|ogg]";
    let cli = Cli::parse(args, &["-o", "--out", "--format"], &[])?;
    let [name] = match cli.positionals.as_slice() {
        [] => {
            print!("{}", tono::audition::preset_list());
            return Ok(());
        }
        [name] => [name],
        more => anyhow::bail!("unexpected argument '{}'\nusage: {usage}", more[1]),
    };
    let preset = tono::audition::find_preset(name)
        .ok_or_else(|| anyhow::anyhow!("unknown preset '{name}' — run 'tono presets' to list"))?;
    let out_dir = PathBuf::from(cli.flag(&["-o", "--out"]).unwrap_or("."));
    let format = parse_format(cli.flag(&["--format"]))?;
    fs::create_dir_all(&out_dir)?;
    let sample_rate = 48_000;
    let (left, right) = tono::audition::bounce_preset(preset, sample_rate)?;
    write_bounce(
        sanitize_stem(preset.name)?.as_str(),
        &out_dir,
        format,
        sample_rate,
        &left,
        &right,
    )
}

/// `tono catalog` — list the catalog voices, or render one's demo.
fn catalog_cmd(args: &[String]) -> anyhow::Result<()> {
    let usage = "tono catalog [NAME] [-o DIR] [--format wav|flac|ogg]";
    let cli = Cli::parse(args, &["-o", "--out", "--format"], &[])?;
    let [slug] = match cli.positionals.as_slice() {
        [] => {
            print!("{}", tono::audition::catalog_list());
            return Ok(());
        }
        [slug] => [slug],
        more => anyhow::bail!("unexpected argument '{}'\nusage: {usage}", more[1]),
    };
    let voice = tono::audition::find_voice(slug)
        .ok_or_else(|| anyhow::anyhow!("unknown voice '{slug}' — run 'tono catalog' to list"))?;
    let out_dir = PathBuf::from(cli.flag(&["-o", "--out"]).unwrap_or("."));
    let format = parse_format(cli.flag(&["--format"]))?;
    fs::create_dir_all(&out_dir)?;
    let doc = tono::audition::voice_demo_doc(&voice)?;
    render_to_dir(&doc, &sanitize_stem(slug)?, &out_dir, format)
}

/// Write a bounced stereo render: the audio file plus the two feedback images
/// and the stats JSON — the same output set as `tono render`, for material
/// that didn't come from a document (the preset auditions).
fn write_bounce(
    stem: &str,
    out_dir: &Path,
    format: &str,
    sample_rate: u32,
    left: &[f32],
    right: &[f32],
) -> anyhow::Result<()> {
    let audio_path = out_dir.join(format!("{stem}.{format}"));
    match format {
        "flac" => tono::audio::write_flac(&audio_path, &[left, right], sample_rate, 16)?,
        "ogg" => tono::audio::write_ogg(&audio_path, &[left, right], sample_rate, 0.7)?,
        _ => tono::audio::write_wav_stereo(&audio_path, left, right, sample_rate, 16)?,
    }
    let mono: Vec<f32> = left.iter().zip(right).map(|(l, r)| 0.5 * (l + r)).collect();
    let png = out_dir.join(format!("{stem}.png"));
    let analysis = tono::imaging::analyze_to_disk(&mono, Some((left, right)), sample_rate, &png)?;
    let stats = out_dir.join(format!("{stem}.stats.json"));
    fs::write(&stats, serde_json::to_string_pretty(&analysis)?)?;

    println!("{}", audio_path.display());
    println!("{}", png.display());
    println!("{}", analysis.waveform_png_path);
    println!("{}", stats.display());
    Ok(())
}

#[cfg(all(test, not(feature = "play")))]
mod play_tests {
    #[test]
    fn play_without_the_feature_says_so() {
        let err = super::play_cmd(&[]).unwrap_err();
        assert!(err.to_string().contains("--features play"));
    }
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
        let cli = Cli::parse(&args(&["-o", "out", "doc.json"]), &["-o", "--out"], &[]).unwrap();
        assert_eq!(cli.flag(&["-o", "--out"]), Some("out"));
        assert_eq!(cli.positionals, vec!["doc.json"]);
    }

    #[test]
    fn flag_aliases_resolve_in_order() {
        let cli = Cli::parse(&args(&["--out", "d", "f.json"]), &["-o", "--out"], &[]).unwrap();
        assert_eq!(cli.flag(&["-o", "--out"]), Some("d"), "long alias found");
        assert_eq!(cli.flag(&["--missing"]), None, "absent flag is None");
    }

    #[test]
    fn unknown_option_is_a_loud_error() {
        let err = Cli::parse(&args(&["--bogus", "x", "f.json"]), &["-o"], &[])
            .err()
            .unwrap();
        assert!(err.to_string().contains("unknown option '--bogus'"));
    }

    #[test]
    fn flag_missing_its_value_is_a_loud_error() {
        let err = Cli::parse(&args(&["f.json", "-o"]), &["-o"], &[])
            .err()
            .unwrap();
        assert!(err.to_string().contains("option '-o' needs a value"));
    }

    #[test]
    fn input_wants_exactly_one_positional() {
        let one = Cli::parse(&args(&["f.json"]), &[], &[]).unwrap();
        assert_eq!(one.input("usage").unwrap(), "f.json");

        let none = Cli::parse(&args(&[]), &[], &[]).unwrap();
        assert!(
            none.input("the-usage")
                .err()
                .unwrap()
                .to_string()
                .contains("the-usage")
        );

        let extra = Cli::parse(&args(&["a.json", "b.json"]), &[], &[]).unwrap();
        let msg = extra.input("usage").err().unwrap().to_string();
        assert!(
            msg.contains("unexpected argument 'b.json'"),
            "names the offender: {msg}"
        );
    }

    #[test]
    fn flag_order_does_not_matter() {
        let before = Cli::parse(&args(&["--format", "ogg", "f.json"]), &["--format"], &[]).unwrap();
        let after = Cli::parse(&args(&["f.json", "--format", "ogg"]), &["--format"], &[]).unwrap();
        assert_eq!(before.flag(&["--format"]), after.flag(&["--format"]));
        assert_eq!(before.positionals, after.positionals);
    }

    #[test]
    fn doc_names_stay_out_of_the_output_path() {
        // A doc name doubles as a file stem — separators and parent refs must
        // be rejected, not written through.
        assert!(sanitize_stem("laser_zap").is_ok());
        assert!(sanitize_stem("../escape").is_err());
        assert!(sanitize_stem("a/b").is_err());
        assert!(sanitize_stem("a\\b").is_err());
        assert!(sanitize_stem("C:evil").is_err());
        assert!(sanitize_stem("..").is_err());
    }

    #[test]
    fn bool_flags_are_valuesless() {
        let cli = Cli::parse(&args(&["--watch", "f.json"]), &[], &["--watch"]).unwrap();
        assert!(cli.has("--watch"));
        assert_eq!(cli.positionals, vec!["f.json"]);
        let cli = Cli::parse(&args(&["f.json"]), &[], &["--watch"]).unwrap();
        assert!(!cli.has("--watch"));
    }

    #[test]
    fn wait_for_change_returns_when_the_mtime_differs() {
        let dir = std::env::temp_dir().join("tono-watch-test");
        std::fs::create_dir_all(&dir).unwrap();
        let f = dir.join("w.json");
        std::fs::write(&f, "{}").unwrap();
        // A `since` older than the file must return immediately, not block.
        let got =
            wait_for_change(f.to_str().unwrap(), Some(std::time::SystemTime::UNIX_EPOCH)).unwrap();
        assert_eq!(got, file_mtime(f.to_str().unwrap()).unwrap());
    }

    #[test]
    fn midi_song_flag_reads_a_song_that_is_not_a_sounddoc() {
        // A Song JSON has no `root` — the doc path must keep rejecting it
        // while --song accepts it.
        let mut song = tono_core::song::Song::new("flagtest", 120.0);
        song.add_track(
            "keys",
            tono_core::dsl::SeqWave::Square,
            tono_core::dsl::Adsr::default(),
        );
        song.tracks[0].notes.push(tono_core::song::note(0, 2, "C4"));
        let dir = std::env::temp_dir().join("tono-cli-song-test");
        fs::create_dir_all(&dir).unwrap();
        let song_path = dir.join("flagtest.json");
        fs::write(&song_path, serde_json::to_string_pretty(&song).unwrap()).unwrap();
        let out = dir.join("flagtest.mid");
        let (song_path, out) = (song_path.to_str().unwrap(), out.to_str().unwrap());

        // The doc path is unchanged: a Song JSON is not a SoundDoc.
        assert!(midi_cmd(&args(&[song_path, "-o", out])).is_err());

        // --song switches the input format.
        midi_cmd(&args(&[song_path, "--song", "-o", out])).unwrap();
        let bytes = fs::read(out).unwrap();
        let smf = midly::Smf::parse(&bytes).unwrap();
        assert_eq!(smf.tracks.len(), 1);
    }

    #[test]
    fn import_song_flag_writes_a_song_json() {
        // A MIDI file made through the export path imports as a Song whose
        // notes sit directly on the tracks.
        let doc: SoundDoc = serde_json::from_str(
            r#"{ "name":"m", "duration":2.0, "root":{ "type":"seq", "bpm":120,
              "steps_per_beat":4, "wave":"square", "env":{"a":0.005,"d":0.1,"s":0.3,"r":0.05},
              "notes":[ {"step":0,"len":2,"pitch":"C4"}, {"step":2,"len":2,"pitch":"E4"} ] } }"#,
        )
        .unwrap();
        let dir = std::env::temp_dir().join("tono-cli-song-test");
        fs::create_dir_all(&dir).unwrap();
        let mid = dir.join("imp.mid");
        tono::midi::export_midi(&doc, &mid).unwrap();
        let out = dir.join("imp.song.json");
        let (mid, out) = (mid.to_str().unwrap(), out.to_str().unwrap());

        import_cmd(&args(&[mid, "--song", "--steps-per-beat", "4", "-o", out])).unwrap();
        let song: tono_core::song::Song =
            serde_json::from_str(&fs::read_to_string(out).unwrap()).unwrap();
        assert_eq!(song.name, "imp", "the song takes the file's stem");
        assert_eq!(song.tracks.len(), 1);
        assert_eq!(song.tracks[0].name, "track_0");
        assert_eq!(song.tracks[0].notes.len(), 2);
        song.to_doc().expect("the imported song compiles");
    }
}
