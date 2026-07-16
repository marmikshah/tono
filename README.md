<p align="center">
  <img src="docs/logo.png" width="112" alt="tono — a pluck waveform on a dark tile">
</p>
<p align="center">
  <img src="docs/logo-wordmark.png" width="384" alt="tono">
</p>

<p align="center"><strong>Game audio as a pure function — procedural, deterministic, zero-asset.<br>Author a synthesis graph; get byte-identical audio from Rust, Python, a CLI, or a live keyboard.</strong></p>

<p align="center">
  <a href="https://github.com/marmikshah/tono/actions/workflows/ci.yml"><img src="https://github.com/marmikshah/tono/actions/workflows/ci.yml/badge.svg" alt="CI"></a>
  <a href="https://crates.io/crates/tono-core"><img src="https://img.shields.io/crates/v/tono-core" alt="crates.io"></a>
  <a href="https://docs.rs/tono-core"><img src="https://img.shields.io/docsrs/tono-core" alt="docs.rs"></a>
  <img src="https://img.shields.io/badge/license-MIT-8c6ee6" alt="license">
</p>

<p align="center">
  <a href="https://marmikshah.github.io/tono/">Showcase</a> ·
  <a href="https://marmikshah.github.io/tono/architecture.html">Architecture</a> ·
  <a href="https://docs.rs/tono-core">API docs</a> ·
  <a href="docs/cookbook.md">Cookbook</a>
</p>

<p align="center">
  <img src="docs/river-flows-spectrogram.png" width="640" alt="spectrogram of River Flows in You, 800 notes on the sampled piano">
</p>

<p align="center"><em>Everything you can hear below was rendered by this engine — no samples, no WAVs shipped.</em></p>

## Hear it

Recognizable classics rebuilt from scratch — the ▶ links play right on GitHub:

| Sound | Play | The trick |
|---|---|---|
| retro-coin | [▶](docs/examples/audio/retro-coin.mp4) | B5 grace note into a held E6 — the interval *is* the sound |
| jump-8bit | [▶](docs/examples/audio/jump-8bit.mp4) | exponential square sweep, gone at sustain 0 |
| waka | [▶](docs/examples/audio/waka.mp4) | per-note pitch slides alternating up/down — the chomp drawn into the notes |
| nokia-tune | [▶](docs/examples/audio/nokia-tune.mp4) | 13 notes of Gran Vals on the Karplus-Strong pluck |
| deep-note | [▶](docs/examples/audio/deep-note.mp4) | supersaw tracks gliding from a scattered cluster onto a five-octave D chord |
| river-flows-in-you | [▶](docs/examples/audio/river-flows-in-you.mp4) | a complete piano piece — 800 notes on the sampled grand |
| band-demo | [▶](docs/examples/audio/band-demo.mp4) | four instruments, one groove, mixed on the stereo bus |

More in [docs/examples/audio/](docs/examples/audio/) — game-ready BGM loops
and ambient beds, all deterministic renders.

## Why tono

- **Sounds are data.** A sound is a JSON synthesis graph; rendering it is a
  pure function → byte-identical audio, every run. Test it, diff it, cache it,
  ship no asset files.
- **Zero-asset SFX.** A patch renders infinite variations from gameplay
  parameters — impacts that scale with collision force, footsteps that vary by
  surface. No sample library.
- **A real game runtime.** Live DSP buses, polyphony caps with priority
  stealing, and adaptive music that reacts on the beat — section switches,
  intensity stems, stingers on the downbeat.
- **An ear built in.** Every render returns a spectrogram, a waveform, and
  LUFS/spectral stats — "does it sound right?" becomes numbers and pictures.

## Quick start

```sh
cargo install tono       # the CLI
cargo add tono-core      # …or the engine as a library
```

```sh
tono render docs/examples/blip.json -o out/
#   out/blip.wav         out/blip.png (spectrogram)
#   out/blip_wave.png    out/blip.stats.json (peak/RMS/LUFS/spectral)
```

That loop — write a doc, render, read the images and stats, refine — is all a
human *or an agent* needs to author sound by inspection. The
[cookbook](docs/cookbook.md) has the full node vocabulary and recipes.

## In a few lines

```rust
use tono_core::adaptive::{AdaptiveMusic, Quantize};

let mut music = AdaptiveMusic::new(48_000);
music.set_tempo(120.0, 4);
music.add_section("explore", &explore_doc);
let battle = music.add_section("battle", &battle_doc);

music.transition_to(battle, Quantize::Bar);  // combat! — swaps on the next bar
music.set_intensity(0.9);                    // stems swell with the action
music.stinger_at(&boss_hit, Quantize::Bar);  // lands on the downbeat
```

```python
import tono

engine = tono.Engine(48000)                  # owns the stream + render thread
engine.drumkit().note_on(36, 1.0)            # kick
engine.load_patch(impact_json).trigger(hardness=0.8, size=0.3)  # zero WAVs
```

More: [embedding & patches](docs/runtime.md) · [API docs](https://docs.rs/tono-core).

## One engine, five faces

Every face renders the same `SoundDoc` byte-identically. New to the codebase?
Start with the [architecture guide](https://marmikshah.github.io/tono/architecture.html).

| Face | What it is | Entry point |
|---|---|---|
| CLI | render → audio + spectrogram + stats | `tono render f.json -o out/` |
| Rust library | the engine embedded in a game or tool | `cargo add tono-core` |
| Python bindings | live engine + deterministic numpy renders | `make python` |
| Pattern station | Tauri app: FL-style step grid, live audio, undo | `make desktop` |
| Playground | hear Rust snippets through the speakers | `make play EXAMPLE=band` |

## A personal note

Every line of code in tono was written by AI — my part was direction, and
holding the project to the standards I use where I still write the code
myself. If tono helps you as a tool, a reference, or a kick-start, that makes
me genuinely happy: the tokens are already spent; the least they can do is be
useful to you too.

> **⚠️ Versions below 2.0.0** are AI-generated and not fully human-reviewed.
> I intend to follow SemVer, but breaking changes may slip into 1.x releases
> despite my best intentions. 2.0.0 — the release where I review everything
> myself — will be tagged by me, by hand; it is the one release an AI agent is
> not allowed to cut. Until then, use at your own risk.

## License

[MIT](LICENSE) — permissive, no warranty.
