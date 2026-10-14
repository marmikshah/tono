# CLI

The `tono` command generates and renders sound effects, compiles songs,
imports/exports MIDI, and writes audio, images, stems, and statistics.

Run `tono --help` and `tono <command> --help` for the arguments and defaults.
From a checkout, use `cargo run --locked -- --help`.

## Commands

| Command | What it does |
| --- | --- |
| `tono templates` | List the eight game SFX starters. |
| `tono generate TEMPLATE` | Generate seeded candidates: editable JSON, audio, spectrogram, waveform, stats, an `index.html` to audition, and `manifest.json`. |
| `tono render FILE.json` | Render a SoundDoc to audio plus the feedback images and stats; `--stems` writes every track and bus stem, `--watch` re-renders on save. |
| `tono vary FILE.json` | Render deterministic round-robin variations of a SoundDoc. |
| `tono schema [TARGET]` | Print the JSON Schema of the `sounddoc` (default) or `patch` document format. |
| `tono compile SONG.json` | Compile a Song into a validated, hashed Program bundle; `--inspect` prints the summary as JSON. |
| `tono presets [NAME]` | List the factory presets, or render one's demo riff. |
| `tono catalog [NAME]` | List the catalog voices, or render one's demo. |
| `tono midi FILE.json` | Export a SoundDoc's sequences (or, with `--song`, a Song) to a Standard MIDI File. Feature `midi`. |
| `tono import FILE.mid` | Import a Standard MIDI File as a SoundDoc (or, with `--song`, a Song). Feature `midi`. |
| `tono play FILE.json` | Audition a SoundDoc through the speakers. Feature `play`. |

Every rendering command takes `-o DIR` and `--format wav|flac|ogg`.

## Features

The default build writes WAV and needs no C toolchain or audio backend.
Cargo features add the rest, at install time
(`cargo install tono --features midi,encode`):

| Feature | Adds |
| --- | --- |
| `midi` | The `midi` and `import` commands (via `midly`). |
| `encode` | `--format flac` (via `flacenc`) and `--format ogg` (via `vorbis_rs`, which compiles libvorbis). Without it those formats fail with an error naming the feature. |
| `play` | The `play` command (speaker playback via `cpal`). |

Start with the [quickstart](../get-started/quickstart.md),
[game SFX generation](../guides/generation.md), or
[SoundDoc reference](sounddoc.md).
