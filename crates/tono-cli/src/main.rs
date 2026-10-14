//! tono — a deterministic sound engine on the command line.
//!
//! Render a `SoundDoc` to audio plus the two feedback images and stats, so any
//! command-line tool can author sound by the loop: write a doc,
//! render it, look at the spectrogram/waveform, refine.

// The doc comments on the clap types are the --help text, and its
// placeholders (<name>.png, <template>-<seed>) are not HTML.
#![allow(rustdoc::invalid_html_tags)]

use std::fs;
use std::path::{Path, PathBuf};

use clap::builder::TypedValueParser;
use clap::{Args, Parser, Subcommand, ValueEnum};
use tono_core::dsl::SoundDoc;
use tono_core::render;

mod audio;
mod audition;
mod compile;
mod generation;
mod imaging;
#[cfg(feature = "midi")]
mod midi;
#[cfg(feature = "play")]
mod play;

const AFTER_HELP: &str = "\
The SoundDoc format and the node vocabulary are documented in the SoundDoc
reference: https://marmikshah.github.io/tono/reference/sounddoc

Optional cargo features (cargo install tono --features midi,encode,play):
  midi    the `midi` and `import` subcommands (Standard MIDI Files)
  encode  --format flac|ogg on the rendering commands (WAV needs no feature)
  play    the `play` subcommand (speaker playback)";

/// A deterministic sound engine.
///
/// Every rendering command writes the audio file next to a spectrogram, a
/// waveform image and a stats JSON, so a sound can be authored by the loop:
/// write a doc, render it, look at the images, refine.
// The doc comment is the about text (first paragraph) and the long about
// (all of it); the Cargo description is too long for a help header.
#[derive(Parser)]
#[command(
    name = "tono",
    version,
    after_help = AFTER_HELP,
    arg_required_else_help = true
)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// List the eight game SFX starters
    Templates,

    /// Generate seeded candidates of a game SFX starter
    ///
    /// Writes COUNT candidates (default 4) with editable JSON, audio, feedback
    /// images, stats, an index.html to audition them, and manifest.json. The
    /// output directory must be empty; the default is
    /// target/generated/<template>-<seed>. Seeds advance by one for each
    /// candidate.
    Generate(generation::GenerateArgs),

    /// Render a SoundDoc to audio, feedback images and stats
    ///
    /// Writes into DIR (default: the current directory):
    ///   <name>.wav|flac|ogg   the audio
    ///   <name>.png            spectrogram   (look at this)
    ///   <name>_wave.png       waveform      (and this)
    ///   <name>.stats.json     peak/RMS/LUFS/spectral/transient analysis
    ///
    /// --stems also writes every track and bus stem (pre-master) as stereo
    /// WAVs into its DIR. --watch re-renders on every save (Ctrl-C to stop):
    /// the edit/inspect loop at full speed.
    #[command(verbatim_doc_comment)]
    Render(RenderArgs),

    /// Render deterministic variations of a SoundDoc
    ///
    /// Round-robin takes of a footstep, impact, pickup: writes
    /// <name>_v<i>.json plus the render outputs for each of the COUNT
    /// variations (default 4, amount 0.15).
    Vary(VaryArgs),

    /// Print the JSON Schema of a document format
    ///
    /// For editor autocomplete and validation.
    Schema(SchemaArgs),

    /// Export a SoundDoc's sequences (or a Song) to a Standard MIDI File
    ///
    /// Each seq becomes one MIDI track. With --song the input is a Song
    /// instead: each song track becomes a MIDI track, the kit on channel 10.
    #[cfg(feature = "midi")]
    Midi(MidiArgs),

    /// Compile a Song into a validated, hashed Program bundle
    ///
    /// Writes <name>.program.json by default; an existing default path is
    /// never overwritten. Every problem is reported in one pass: on failure
    /// each diagnostic prints with its code, path, and fix, and the exit code
    /// is non-zero.
    Compile(CompileArgs),

    /// Import a Standard MIDI File as a renderable SoundDoc (or a Song)
    ///
    /// Produces seq tracks: GM programs map to the built-in voices and
    /// channel 10 becomes the drum kit. With --song the output is a Song
    /// instead: notes land directly on the tracks, no patterns.
    #[cfg(feature = "midi")]
    Import(ImportArgs),

    /// Audition a SoundDoc through the speakers
    #[cfg(feature = "play")]
    Play(PlayArgs),

    /// List the 16 factory presets, or render one's demo riff
    ///
    /// Without NAME, lists the presets. With NAME, renders the preset's demo
    /// riff (a C-major arpeggio through the live Instrument engine) with the
    /// same outputs as `tono render`.
    Presets(PresetsArgs),

    /// List the 31 catalog voices, or render one's demo
    ///
    /// Without NAME, lists the voices by family. With NAME, renders the
    /// voice's demo: a C-major scale resolving to a chord (a two-bar groove
    /// for the drum kits).
    Catalog(CatalogArgs),
}

/// The audio container a rendering command writes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
enum Format {
    /// 16-bit PCM WAV; loop docs get a `smpl` loop chunk
    Wav,
    /// FLAC, lossless (needs the `encode` feature)
    Flac,
    /// OGG Vorbis, lossy VBR (needs the `encode` feature)
    Ogg,
}

impl Format {
    fn extension(self) -> &'static str {
        match self {
            Format::Wav => "wav",
            Format::Flac => "flac",
            Format::Ogg => "ogg",
        }
    }

    /// A build without the encoders rejects the compressed formats at parse
    /// time, naming the feature — before any output directory is created.
    fn ensure_encodable(self) -> Result<Self, String> {
        if self != Format::Wav && !cfg!(feature = "encode") {
            return Err(
                "tono was built without the encode feature; reinstall with --features encode"
                    .to_string(),
            );
        }
        Ok(self)
    }
}

/// The `--format` value parser: the enum's values, plus the feature check.
fn format_parser() -> impl TypedValueParser<Value = Format> {
    clap::builder::EnumValueParser::<Format>::new().try_map(Format::ensure_encodable)
}

/// A `0..1` control value (brightness, punch, variation, vary amount).
fn unit_interval(s: &str) -> Result<f32, String> {
    let value: f32 = s.parse().map_err(|_| format!("'{s}' is not a number"))?;
    if !(0.0..=1.0).contains(&value) {
        return Err(format!("{value} is not in 0..1"));
    }
    Ok(value)
}

/// Where a rendering command writes, and in which format.
#[derive(Args)]
struct OutputArgs {
    /// Output directory (created if missing)
    #[arg(short = 'o', long = "out", value_name = "DIR", default_value = ".")]
    out: PathBuf,

    /// Audio file format
    #[arg(long, value_name = "FORMAT", default_value = "wav", value_parser = format_parser())]
    format: Format,
}

#[derive(Args)]
struct RenderArgs {
    /// The SoundDoc to render
    #[arg(value_name = "FILE.json")]
    file: String,

    #[command(flatten)]
    output: OutputArgs,

    /// Also write every track and bus stem (pre-master) as stereo WAVs into DIR
    #[arg(long, value_name = "DIR")]
    stems: Option<PathBuf>,

    /// Re-render on every save (Ctrl-C to stop)
    #[arg(long)]
    watch: bool,
}

#[derive(Args)]
struct VaryArgs {
    /// The SoundDoc to vary
    #[arg(value_name = "FILE.json")]
    file: String,

    /// How many variations to render
    #[arg(short = 'n', long = "count", value_name = "COUNT", default_value_t = 4,
          value_parser = clap::value_parser!(u32).range(1..=256))]
    count: u32,

    /// Mutation amount
    #[arg(long, value_name = "0..1", default_value_t = 0.15, value_parser = unit_interval)]
    amount: f32,

    /// Seed; variation i renders with seed + i
    #[arg(long, value_name = "N", default_value_t = 0)]
    seed: u64,

    #[command(flatten)]
    output: OutputArgs,
}

/// The document formats `tono schema` can print.
#[derive(Clone, Copy, ValueEnum)]
enum SchemaTarget {
    /// The SoundDoc (sound, song and instrument documents)
    Sounddoc,
    /// The Patch applied to a document
    Patch,
}

#[derive(Args)]
struct SchemaArgs {
    /// Which document format
    #[arg(value_enum, default_value = "sounddoc")]
    target: SchemaTarget,
}

#[cfg(feature = "midi")]
#[derive(Args)]
struct MidiArgs {
    /// The SoundDoc (or, with --song, the Song) to export
    #[arg(value_name = "FILE.json")]
    file: String,

    /// Output MIDI file [default: out.mid, never overwritten]
    #[arg(short = 'o', long = "out", value_name = "FILE.mid")]
    out: Option<PathBuf>,

    /// Read a Song instead of a SoundDoc
    #[arg(long)]
    song: bool,
}

#[derive(Args)]
struct CompileArgs {
    /// The Song to compile
    #[arg(value_name = "SONG.json")]
    file: String,

    /// Output bundle [default: <name>.program.json, never overwritten]
    #[arg(short = 'o', long = "out", value_name = "FILE")]
    out: Option<PathBuf>,

    /// Compile for this sample rate instead of the song's
    #[arg(long, value_name = "N", value_parser = clap::value_parser!(u32).range(1..))]
    sample_rate: Option<u32>,

    /// Print the machine-readable summary (hash, version pins, track roster,
    /// resource estimates, warnings) as JSON and write nothing
    #[arg(long)]
    inspect: bool,
}

#[cfg(feature = "midi")]
#[derive(Args)]
struct ImportArgs {
    /// The Standard MIDI File to import
    #[arg(value_name = "FILE.mid")]
    file: String,

    /// Output document [default: the MIDI file's stem + .json, never overwritten]
    #[arg(short = 'o', long = "out", value_name = "DOC.json")]
    out: Option<PathBuf>,

    /// Grid resolution the notes quantize to
    #[arg(long, value_name = "N", default_value_t = 4,
          value_parser = clap::value_parser!(u32).range(1..=64))]
    steps_per_beat: u32,

    /// Write a Song instead of a SoundDoc
    #[arg(long)]
    song: bool,
}

#[cfg(feature = "play")]
#[derive(Args)]
struct PlayArgs {
    /// The SoundDoc to play
    #[arg(value_name = "FILE.json")]
    file: String,

    /// Seconds to play [default: the doc's duration + 1]
    #[arg(long, value_name = "N")]
    secs: Option<f32>,
}

#[derive(Args)]
struct PresetsArgs {
    /// The preset to render; omit to list them
    #[arg(value_name = "NAME")]
    name: Option<String>,

    #[command(flatten)]
    output: OutputArgs,
}

#[derive(Args)]
struct CatalogArgs {
    /// The voice to render; omit to list them
    #[arg(value_name = "NAME")]
    name: Option<String>,

    #[command(flatten)]
    output: OutputArgs,
}

fn main() -> anyhow::Result<()> {
    match Cli::parse().command {
        Commands::Templates => generation::templates_cmd(),
        Commands::Generate(args) => generation::generate_cmd(args),
        Commands::Render(args) => render_cmd(args),
        Commands::Vary(args) => vary_cmd(args),
        Commands::Schema(args) => schema_cmd(args),
        #[cfg(feature = "midi")]
        Commands::Midi(args) => midi_cmd(args),
        Commands::Compile(args) => compile_cmd(args),
        #[cfg(feature = "midi")]
        Commands::Import(args) => import_cmd(args),
        #[cfg(feature = "play")]
        Commands::Play(args) => play_cmd(args),
        Commands::Presets(args) => presets_cmd(args),
        Commands::Catalog(args) => catalog_cmd(args),
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

fn render_cmd(args: RenderArgs) -> anyhow::Result<()> {
    let RenderArgs {
        file,
        output: OutputArgs {
            out: out_dir,
            format,
        },
        stems: stems_dir,
        watch,
    } = args;
    fs::create_dir_all(&out_dir)?;
    if let Some(dir) = &stems_dir {
        fs::create_dir_all(dir)?;
    }

    // Render once, then re-render on every save. With --watch, an edit that
    // leaves the doc invalid is reported and watched through, not fatal.
    let mut since: Option<std::time::SystemTime> = None;
    loop {
        let result = load_doc(&file).and_then(|doc| {
            let stem = if doc.name.is_empty() {
                Path::new(&file)
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
            &file,
            since.or_else(|| file_mtime(&file).ok()),
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

/// Write a stereo pair as `<stem>.<format>` and return the path.
fn write_audio(
    out_dir: &Path,
    stem: &str,
    format: Format,
    left: &[f32],
    right: &[f32],
    sample_rate: u32,
) -> anyhow::Result<PathBuf> {
    let path = out_dir.join(format!("{stem}.{}", format.extension()));
    match format {
        Format::Wav => crate::audio::write_wav_stereo(&path, left, right, sample_rate, 16)?,
        #[cfg(feature = "encode")]
        Format::Flac => crate::audio::write_flac(&path, &[left, right], sample_rate, 16)?,
        #[cfg(feature = "encode")]
        Format::Ogg => crate::audio::write_ogg(&path, &[left, right], sample_rate, 0.7)?,
        // The value parser already refused these on a build without encoders.
        #[cfg(not(feature = "encode"))]
        Format::Flac | Format::Ogg => {
            format.ensure_encodable().map_err(anyhow::Error::msg)?;
        }
    }
    Ok(path)
}

/// The full render pipeline for one doc: audio file (+ `smpl` chunk for loop
/// docs), the two feedback images, and the stats JSON — printing each output
/// path. Shared by `render`, `vary`, and `generate`.
fn render_to_dir(doc: &SoundDoc, stem: &str, out_dir: &Path, format: Format) -> anyhow::Result<()> {
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

    let audio_path = write_audio(out_dir, stem, format, left, right, doc.sample_rate)?;
    // A `loop` doc's WAV carries a `smpl` chunk spanning the whole rendered
    // loop body, so game engines loop at the sample-accurate points.
    if format == Format::Wav
        && matches!(doc.playback, tono_core::dsl::Playback::Loop { .. })
        && !left.is_empty()
    {
        crate::audio::append_smpl_loop(
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
    let analysis = crate::imaging::analyze_to_disk(&product.mono, stereo, doc.sample_rate, &png)?;
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
        crate::audio::write_wav_stereo(&path, &s.left, &s.right, doc.sample_rate, 16)?;
        println!("{}", path.display());
    }
    Ok(())
}

/// `tono vary` — deterministic round-robin variations of one document.
fn vary_cmd(args: VaryArgs) -> anyhow::Result<()> {
    let VaryArgs {
        file,
        count,
        amount,
        seed,
        output: OutputArgs {
            out: out_dir,
            format,
        },
    } = args;
    let doc = load_doc(&file)?;
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
fn schema_cmd(args: SchemaArgs) -> anyhow::Result<()> {
    let schema = match args.target {
        SchemaTarget::Sounddoc => schemars::schema_for!(SoundDoc),
        SchemaTarget::Patch => schemars::schema_for!(tono_core::patch::Patch),
    };
    println!("{}", serde_json::to_string_pretty(&schema)?);
    Ok(())
}

#[cfg(feature = "midi")]
fn midi_cmd(args: MidiArgs) -> anyhow::Result<()> {
    let out = match args.out {
        Some(o) => o,
        // A defaulted output must never silently clobber an existing file.
        None if Path::new("out.mid").exists() => {
            anyhow::bail!("out.mid already exists — pass -o to choose a different output")
        }
        None => PathBuf::from("out.mid"),
    };
    // --song switches the input format: a Song lowers through to_doc and its
    // tracks become the MIDI tracks; without it the input is a SoundDoc.
    let summary = if args.song {
        let song: tono_core::song::Song = serde_json::from_str(&fs::read_to_string(&args.file)?)
            .map_err(|e| anyhow::anyhow!("parsing {} as a Song: {e}", args.file))?;
        crate::midi::export_song_midi(&song, &out)?
    } else {
        let doc = load_doc(&args.file)?;
        crate::midi::export_midi(&doc, &out)?
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
fn compile_cmd(args: CompileArgs) -> anyhow::Result<()> {
    let program = crate::compile::compile_song(&args.file, args.sample_rate)?;
    for w in &program.warnings() {
        eprintln!("{} {} {}: {}", w.severity, w.code, w.path, w.message);
    }
    if args.inspect {
        let inspect = crate::compile::inspect_json(&program);
        println!("{}", serde_json::to_string_pretty(&inspect)?);
        return Ok(());
    }
    let out = match args.out {
        Some(o) => o,
        None => {
            let stem = if program.doc.name.is_empty() {
                Path::new(&args.file)
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
#[cfg(feature = "midi")]
fn import_cmd(args: ImportArgs) -> anyhow::Result<()> {
    let file = Path::new(&args.file);
    let spb = args.steps_per_beat;
    let out = match args.out {
        Some(o) => o,
        None => {
            // A defaulted output must never silently clobber an existing file
            // (e.g. an authored doc sharing the MIDI file's stem).
            let default = file.with_extension("json");
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
    if args.song {
        let song = crate::midi::import_midi_song(file, spb)?;
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
        let (doc, summary) = crate::midi::import_midi(file, spb)?;
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

/// `tono play` — audition a doc through the speakers (feature `play`).
#[cfg(feature = "play")]
fn play_cmd(args: PlayArgs) -> anyhow::Result<()> {
    let doc = load_doc(&args.file)?;
    let secs = args.secs.unwrap_or(doc.duration + 1.0);
    crate::play::play_doc(&doc, secs)
}

/// `tono presets` — list the factory presets, or render one's demo riff.
fn presets_cmd(args: PresetsArgs) -> anyhow::Result<()> {
    let Some(name) = args.name else {
        print!("{}", crate::audition::preset_list());
        return Ok(());
    };
    let preset = crate::audition::find_preset(&name)
        .ok_or_else(|| anyhow::anyhow!("unknown preset '{name}' — run 'tono presets' to list"))?;
    let OutputArgs {
        out: out_dir,
        format,
    } = args.output;
    fs::create_dir_all(&out_dir)?;
    let sample_rate = 48_000;
    let (left, right) = crate::audition::bounce_preset(preset, sample_rate)?;
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
fn catalog_cmd(args: CatalogArgs) -> anyhow::Result<()> {
    let Some(slug) = args.name else {
        print!("{}", crate::audition::catalog_list());
        return Ok(());
    };
    let voice = crate::audition::find_voice(&slug)
        .ok_or_else(|| anyhow::anyhow!("unknown voice '{slug}' — run 'tono catalog' to list"))?;
    let OutputArgs {
        out: out_dir,
        format,
    } = args.output;
    fs::create_dir_all(&out_dir)?;
    let doc = crate::audition::voice_demo_doc(&voice)?;
    render_to_dir(&doc, &sanitize_stem(&slug)?, &out_dir, format)
}

/// Write a bounced stereo render: the audio file plus the two feedback images
/// and the stats JSON — the same output set as `tono render`, for material
/// that didn't come from a document (the preset auditions).
fn write_bounce(
    stem: &str,
    out_dir: &Path,
    format: Format,
    sample_rate: u32,
    left: &[f32],
    right: &[f32],
) -> anyhow::Result<()> {
    let audio_path = write_audio(out_dir, stem, format, left, right, sample_rate)?;
    let mono: Vec<f32> = left.iter().zip(right).map(|(l, r)| 0.5 * (l + r)).collect();
    let png = out_dir.join(format!("{stem}.png"));
    let analysis = crate::imaging::analyze_to_disk(&mono, Some((left, right)), sample_rate, &png)?;
    let stats = out_dir.join(format!("{stem}.stats.json"));
    fs::write(&stats, serde_json::to_string_pretty(&analysis)?)?;

    println!("{}", audio_path.display());
    println!("{}", png.display());
    println!("{}", analysis.waveform_png_path);
    println!("{}", stats.display());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    fn parse(args: &[&str]) -> Result<Cli, clap::Error> {
        Cli::try_parse_from(std::iter::once("tono").chain(args.iter().copied()))
    }

    #[test]
    fn command_definition_is_consistent() {
        // clap's own checks on the derive: conflicting ids, bad defaults,
        // required-group mistakes — found at test time, not at first run.
        Cli::command().debug_assert();
    }

    #[test]
    fn flags_consume_their_values() {
        // "-o out" must never leave "out" behind as a positional — a real
        // bug class the previous hand-rolled parser was rewritten to kill.
        let Commands::Render(args) = parse(&["render", "-o", "out", "doc.json"]).unwrap().command
        else {
            panic!("render subcommand");
        };
        assert_eq!(args.file, "doc.json");
        assert_eq!(args.output.out, PathBuf::from("out"));
        assert_eq!(args.output.format, Format::Wav, "format defaults to wav");
        assert!(args.stems.is_none());
        assert!(!args.watch);
    }

    #[test]
    fn long_aliases_and_bool_flags_parse() {
        let Commands::Render(args) =
            parse(&["render", "--out", "d", "--watch", "--stems", "s", "f.json"])
                .unwrap()
                .command
        else {
            panic!("render subcommand");
        };
        assert_eq!(args.output.out, PathBuf::from("d"));
        assert_eq!(args.stems.as_deref(), Some(Path::new("s")));
        assert!(args.watch);
    }

    #[test]
    fn flag_order_does_not_matter() {
        let before = parse(&["vary", "--seed", "7", "-n", "2", "f.json"]).unwrap();
        let after = parse(&["vary", "f.json", "-n", "2", "--seed", "7"]).unwrap();
        for cli in [before, after] {
            let Commands::Vary(args) = cli.command else {
                panic!("vary subcommand");
            };
            assert_eq!(
                (args.file.as_str(), args.seed, args.count),
                ("f.json", 7, 2)
            );
            assert_eq!(args.amount, 0.15, "amount keeps its default");
        }
    }

    #[test]
    fn bad_input_is_a_loud_error() {
        assert!(parse(&["render", "--bogus", "x", "f.json"]).is_err());
        assert!(parse(&["render", "f.json", "-o"]).is_err(), "missing value");
        assert!(parse(&["render"]).is_err(), "missing input");
        assert!(
            parse(&["render", "a.json", "b.json"]).is_err(),
            "extra input"
        );
        assert!(
            parse(&["vary", "f.json", "-n", "0"]).is_err(),
            "count range"
        );
        assert!(parse(&["vary", "f.json", "--amount", "1.5"]).is_err());
        assert!(parse(&["vary", "f.json", "--amount", "NaN"]).is_err());
        assert!(parse(&["render", "f.json", "--format", "mp3"]).is_err());
        assert!(parse(&["schema", "midi"]).is_err());
        assert!(parse(&["nonsense"]).is_err());
    }

    #[cfg(not(feature = "encode"))]
    #[test]
    fn compressed_formats_name_the_missing_feature() {
        let msg = match parse(&["render", "f.json", "--format", "flac"]) {
            Err(err) => err.to_string(),
            Ok(_) => panic!("flac parsed on a build without encoders"),
        };
        assert!(
            msg.contains("built without the encode feature") && msg.contains("--features encode"),
            "{msg}"
        );
        assert!(parse(&["presets", "pad", "--format", "ogg"]).is_err());
    }

    #[cfg(feature = "encode")]
    #[test]
    fn compressed_formats_parse_with_the_feature() {
        let Commands::Render(args) = parse(&["render", "f.json", "--format", "flac"])
            .unwrap()
            .command
        else {
            panic!("render subcommand");
        };
        assert_eq!(args.output.format, Format::Flac);
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

    #[cfg(feature = "midi")]
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
        let args = |song: bool| MidiArgs {
            file: song_path.to_str().unwrap().to_string(),
            out: Some(out.clone()),
            song,
        };

        // The doc path is unchanged: a Song JSON is not a SoundDoc.
        assert!(midi_cmd(args(false)).is_err());

        // --song switches the input format.
        midi_cmd(args(true)).unwrap();
        let bytes = fs::read(&out).unwrap();
        let smf = midly::Smf::parse(&bytes).unwrap();
        assert_eq!(smf.tracks.len(), 1);
    }

    #[cfg(feature = "midi")]
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
        crate::midi::export_midi(&doc, &mid).unwrap();
        let out = dir.join("imp.song.json");

        import_cmd(ImportArgs {
            file: mid.to_str().unwrap().to_string(),
            out: Some(out.clone()),
            steps_per_beat: 4,
            song: true,
        })
        .unwrap();
        let song: tono_core::song::Song =
            serde_json::from_str(&fs::read_to_string(&out).unwrap()).unwrap();
        assert_eq!(song.name, "imp", "the song takes the file's stem");
        assert_eq!(song.tracks.len(), 1);
        assert_eq!(song.tracks[0].name, "track_0");
        assert_eq!(song.tracks[0].notes.len(), 2);
        song.to_doc().expect("the imported song compiles");
    }
}
