# Generate game sound effects

Choose a starter and generate a few candidates without writing a synthesis
graph. The generator uses authored recipes with small seeded changes to
pitch and timing. It needs no sample pack or remote service.

From a checkout:

```sh
cargo run --locked -- templates
cargo run --locked -- generate laser --seed 42
```

The second command writes four candidates to
`target/generated/laser-42/`. Open `index.html` in a browser to audition
the candidates and keep their audio or source files.
Every candidate includes editable JSON, a spectrogram, a waveform, and
numeric analysis. `manifest.json` records its generation spec and filenames.

## Starters

For a larger ready-to-use collection, open the [sound library](/sounds):
64 authored designs across nine categories, each with Classic, Soft, Bright,
Low, High and Long variations. Search by name or tag, preview in your browser,
and download a stereo WAV or its editable JSON recipe. Ambience entries loop
until paused; their WAV exports include a whole-buffer sampler loop.
The site synthesizes these sounds locally with Tono's Rust engine compiled
to WebAssembly. The recipes and generated audio are MIT licensed.

The CLI starter generator below remains useful for seeded game-SFX batches.

| Template | Character |
|---|---|
| `coin` | Bright rising pickup chime |
| `jump` | Upward arcade pitch sweep |
| `laser` | Descending sci-fi zap |
| `explosion` | Noise blast with a low-frequency body |
| `impact` | Short resonant strike |
| `footstep` | Muted contact thud and surface noise |
| `ui-confirm` | Soft ascending confirmation |
| `ui-cancel` | Soft descending cancellation |

## Shape a batch

```sh
tono generate footstep --seed 7 --count 8 \
  --brightness 0.3 --punch 0.7 --variation 0.2 \
  --sample-rate 48000 --format wav -o target/footsteps
```

- `--brightness` changes tone height, filter openness, and strike hardness.
- `--punch` changes attack speed and transient emphasis.
- `--variation` controls how far pitch and timing move between seeds.

All three controls accept finite values from 0 to 1. Brightness and punch
default to 0.5; variation defaults to 0.15. At zero variation, recipe
parameters stay fixed; noise-based starters still use their synthesis seed.
The default seed is 0. Candidate seeds increase by one, wrapping after the
largest unsigned 64-bit integer.

The sample rate defaults to 48 kHz and accepts 8–192 kHz. A batch contains
1–32 candidates, defaulting to four. Choose WAV, FLAC, or OGG with `--format`.
The output directory must be empty. To try another batch, choose a new
directory or a different seed; existing exports are preserved.

## Keep and edit

Keep a candidate's audio and JSON together. To edit its graph and render it
again:

```sh
tono render target/generated/laser-42/laser_v0.json \
  --watch -o target/edited-laser
```

With the optional playback feature installed, preview the document directly:

```sh
tono play target/generated/laser-42/laser_v0.json
```

The manifest stores each candidate's template revision, controls, seed,
sample rate, and document hash. Generation is deterministic for a supported
recipe revision. Save the document as well: its graph and engine revision
preserve the exact sound even when new recipes are introduced.

Generated SFX are short mono one-shots, which a game can position on its own
audio stage. Their envelopes include quiet endpoints. Listen to the results
at the intended game volume and adjust the mix for the scene.

## Generate from Rust

```rust
use tono_core::generate::{SfxSpec, SfxTemplate, generate_sfx};

let mut spec = SfxSpec::new(SfxTemplate::Impact, 7);
spec.brightness = 0.3;
spec.punch = 0.8;
let doc = generate_sfx(&spec)?;
let audio = tono_core::render::render(&doc);
```

`SfxSpec` supports serde serialization. `generate_sfx` validates the request
and returns an ordinary `SoundDoc`; the core performs no file or device I/O
and works with default features disabled. Existing renderers and runtimes
can consume the document.

The larger library is also available from Rust, with discovery metadata in
`tono_core::library::RECIPES` and named voicings in `VARIANTS`:

```rust
use tono_core::library::{LibrarySpec, LibraryVariant, generate};

let mut spec = LibrarySpec::new("metal-clang", LibraryVariant::Soft, 42);
spec.pitch_semitones = -5.0;
spec.duration_scale = 1.4;
let doc = generate(&spec)?;
let audio = tono_core::player::render_stereo(&doc);
```

Library specs include an explicit recipe revision, seed, sample rate, pitch,
duration, brightness, punch and variation. Generated documents contain named
layers and current schema/engine pins. Save the SoundDoc to preserve the sound
across future recipe changes; it works with the existing CLI and runtime.

The [background music library](/bgm) provides original scene music loops, and
the [Sound Studio](/create) lets you build layered sounds in your browser.
Desktop candidate audition is planned in [the game-audio proposal](https://github.com/marmikshah/tono/issues/63).
