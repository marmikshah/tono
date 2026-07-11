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
  <img src="https://img.shields.io/badge/license-MIT-8c6ee6" alt="license">
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

More in [docs/examples/audio/](docs/examples/audio/) — game-ready BGM loops and ambient beds, all deterministic renders.

## What it is

A sound in tono is a **symbolic graph** (oscillators → envelopes → filters →
effects → mix); rendering it is a **pure function of `(graph, seed, sample_rate)`
→ byte-identical audio**. The same document sounds identical offline, streamed
in real time, or played as an instrument — so audio becomes something you can
**test, diff, cache, and ship without asset files**.

- **Instruments & songs** — a polyphonic sequencer, an 11-preset factory bank, a
  GM drum kit, catalog voices (piano, bass, guitar, strings…), and a fluent
  `Song` API that compiles to a plain document.
- **Game runtime** — an embeddable `Engine`/`Mixer` with **live DSP buses**
  (reverb/EQ/compressor inserts + sends), **voice management** (polyphony caps,
  priority stealing), and **interactive music** (beat-quantized section switches,
  intensity layers, stingers on the downbeat).
- **Zero-asset SFX** — a `Patch` renders infinite per-instance variations from
  named parameters (`hardness`, `size`, …). Impact sounds that scale with
  collision force, no sample library.
- **An ear for critique** — LUFS/peak/spectral stats plus spectrogram + waveform
  images per render, so "does it sound right?" becomes numbers and pictures.
- **SoundFont sampler** — point the `sampler` voice at any free GM bank for real
  recorded instruments, still deterministic.

## Use it from the command line

```sh
tono render docs/examples/blip.json -o out/
#   out/blip.wav          the audio            (--format wav|flac|ogg)
#   out/blip.png          the spectrogram      ← look at this
#   out/blip_wave.png     the waveform         ← and this
#   out/blip.stats.json   peak / RMS / LUFS / spectral / transient analysis
```

That loop — emit a doc, render, read the images + stats, refine — is all a
human *or an agent* needs to author sound by inspection.

## Use it from Rust

```rust
use tono_core::dsl::SoundDoc;
use tono_core::render;

let doc: SoundDoc = serde_json::from_str(r#"{
    "name": "blip", "duration": 0.3, "engine": 4,
    "root": { "type": "mul", "inputs": [
        { "type": "sine", "freq": 880 },
        { "type": "env", "a": 0.002, "d": 0.08, "s": 0.0, "r": 0.05 } ] }
}"#)?;

let samples: Vec<f32> = render::render(&doc); // byte-identical every run
```

### Game audio, in a few lines each

```rust
use tono_core::runtime::{Engine, Mixer, Priority};

// Voice management: a budget + priorities, and a bullet-hell scene stays clean.
let mut engine = Engine::new(48_000);
engine.set_max_voices(32);
engine.play_looping_prioritized(music, Priority::CRITICAL); // never stolen
engine.play_prioritized(explosion, Priority::HIGH);
engine.play(footstep);                                      // stolen first

// Live DSP buses: inserts and sends, reusing the streaming effect kernels.
let mut mixer = Mixer::new_at(48_000);
let sfx = mixer.bus("sfx");
let reverb = mixer.fx_bus("reverb", vec![reverb_node])?;
mixer.set_send(sfx, reverb, 0.9);
```

```rust
use tono_core::adaptive::{AdaptiveMusic, Quantize};

// Interactive music: react on the beat, like a film score.
let mut music = AdaptiveMusic::new(48_000);
music.set_tempo(120.0, 4);
music.add_section("explore", &explore_doc);
let battle = music.add_section("battle", &battle_doc);
music.transition_to(battle, Quantize::Bar);  // swaps on the next bar
music.set_intensity(0.9);                    // stems swell with the action
music.stinger_at(&boss_hit, Quantize::Bar);  // lands on the downbeat
```

## Use it from Python

The same engine as a Python extension (`crates/tono-py`) — live procedural
audio for Pygame / Ren'Py / Arcade, or deterministic numpy renders for anything:

```python
import tono

engine = tono.Engine(48000)            # owns the output stream + render thread
engine.drumkit().note_on(36, 1.0)      # kick — the audio thread never touches Python
engine.instrument("warm_lead").note_on("C4", 0.9)

impact = engine.load_patch(open("impact.patch.json").read())
impact.trigger(hardness=0.8, size=0.3) # zero baked WAVs

# …or pull deterministic numpy arrays (testable in CI):
buf = tono.Patch(open("impact.patch.json").read()).render(hardness=0.7)
```

Build it with `make python` (maturin; abi3 wheels for 3.9+ build in CI).

## Install

```sh
cargo add tono-core      # the engine, as a library (games, tools)
cargo install tono       # the `tono render` CLI
```

Sampled instruments need a free General MIDI SoundFont once (FluidR3 GM,
GeneralUser GS): `wave: "sampler", sf2: "/path/to/gm.sf2", sf2_preset: 0`.

## The other faces

One `SoundDoc`, rendered byte-identically by every face:

- **A native pattern station** (`make desktop`) — a Tauri app with real-time
  audio: an FL-style step grid over the catalog instruments, click-free live
  editing, per-track faders, undo, and LUFS/spectrogram feedback per edit.
- **A programmatic playground** (`make play`) — hear a sound, instrument, song,
  bus chain, or adaptive-music arc from a few lines of Rust
  (`make play EXAMPLE=buses` / `voices` / `interactive_music`).

## Determinism

The real-time streaming path is byte-identical to an offline bounce (verified
by a fuzzer in CI), and byte-changing kernel upgrades are gated behind a
document `engine` revision, so old sounds never change. A golden corpus pins
representative renders in CI. Render the same document twice, get the same bytes
— which is what makes audio testable, diffable, and cacheable.

## More

- [docs/cookbook.md](docs/cookbook.md) — the `SoundDoc` DSL, node vocabulary,
  instrument table, worked recipes (validated in CI).
- [docs/runtime.md](docs/runtime.md) — embedding the engine + parametric patches.
- [docs/ROADMAP.md](docs/ROADMAP.md) — where 2.0 is headed.
- `make help` — every target; `make verify` mirrors CI.

## License

[MIT](LICENSE) — permissive, no warranty.
