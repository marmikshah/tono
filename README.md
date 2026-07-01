<p align="center">
  <img src="docs/logo.png" width="112" alt="tono — a pluck waveform on a dark tile">
</p>
<p align="center">
  <img src="docs/logo-wordmark.png" width="384" alt="tono">
</p>

<p align="center"><strong>A deterministic sound engine — author a synthesis graph from code, an AI agent (MCP), or a live keyboard, and get byte-identical audio everywhere.</strong></p>

<p align="center">
  <a href="https://github.com/marmikshah/tono/actions/workflows/ci.yml"><img src="https://github.com/marmikshah/tono/actions/workflows/ci.yml/badge.svg" alt="CI"></a>
  <a href="https://github.com/marmikshah/tono/releases/latest"><img src="https://img.shields.io/github/v/release/marmikshah/tono" alt="release"></a>
  <img src="https://img.shields.io/badge/license-MIT-8c6ee6" alt="license">
</p>

<p align="center">
  <img src="docs/river-flows-spectrogram.png" width="640" alt="spectrogram of River Flows in You, 800 notes on the sampled piano">
</p>
<p align="center">
  <img src="docs/river-flows-waveform.png" width="320" alt="waveform of the piano piece">
  <img src="docs/band-demo-waveform.png" width="320" alt="waveform of the band demo groove">
</p>

<p align="center"><em>Every sound behind these images — a complete piano piece,
a four-instrument band, three game-ready BGM loops — was composed, mixed and mastered by
agents through the MCP tools, and every one replays byte-identically from a
session file in this repo. The logo and wordmark were drawn by an agent with
<a href="https://github.com/marmikshah/atelier">atelier</a>, tono's
pixel-art sibling.</em></p>

## What it is

Agents are good at *describing* sound and bad at *hearing* it. tono closes
the loop: every synth, instrument, and mixer move is a tool call, and every
render hands back analysis — levels, loudness, spectral centroid, transients —
plus a **spectrogram and a waveform image** the agent can actually look at,
judge, and correct. The same listen-and-fix loop a human runs in a DAW. One
Rust binary; no API keys, no network, fully deterministic.

- **A real studio, headless** — a polyphonic sequencer with a core instrument
  set (piano, e-piano, organ, strings, bass, a GM-mapped drum kit, pitched
  cowbell, plucked string, FM mallets) plus raw band-limited oscillators, FM,
  supersaw and three noise colours for synthesis and SFX.
- **Real recorded instruments** — the `sampler` voice plays any SoundFont
  (point it at a free GM bank): sampled grands, basses, string ensembles, GM
  drums. Renders stay deterministic.
- **A mixing console** — per-track pan/gain onto a true stereo bus, a master
  processor chain, decorrelated reverb tails, sidechain ducking, swing and
  humanize groove.
- **An ear for critique** — peak/true-peak/RMS/crest, ≈LUFS, spectral
  centroid, attack/decay/onset/silence descriptors, `compare_sounds` deltas:
  "does it sound right?" becomes numbers an agent can act on.
- **Deterministic, replayable music** — a session file is the ordered journal
  of tool calls; replaying it reproduces the project **byte-for-byte**.
  Annotated example recipes double as tutorials and CI tests.
- **Game-ready output** — WAV/FLAC/OGG, seamless loops with `smpl` chunks,
  loudness-matched packs with `sounds.json` manifests, and engine files for
  Godot / Unity / Bevy.

The full tool surface (24 tools) is documented in [docs/TOOLS.md](docs/TOOLS.md).

## Design by hand, play it, ship it into a game

Beyond the agent loop, the same deterministic engine drives more faces — one
`SoundDoc`, rendered byte-identically by all of them:

- **A real-time runtime** — a byte-identical **streaming renderer** and an
  embeddable `Engine`/`Mixer`, so the same patch that was authored offline plays
  live, block-by-block, driven by gameplay. Ship a parametric
  [patch](docs/runtime.md) and render endless per-instance SFX variations (an
  impact that scales with collision force, a footstep by surface) with **zero
  baked files** — the pure core compiles straight into your game.
- **Playable instruments** — turn any patch into a polyphonic, pitched, gated
  **instrument** and play it with note-on/off, velocity, a sustain pedal, and a
  shared master reverb.
- **A native desktop studio** — a Tauri app with real-time audio: **play your
  patch like an instrument** from the computer keyboard or a MIDI controller
  (`make desktop`). Optional — never part of the default build, MCP server, or CI.
- **A programmatic playground** — build a sound or instrument in a few lines of
  Rust and hear it (`make play`).

## Use it from code

`tono-core` is the published engine crate — a pure, deterministic library. Add
it and render a graph to samples (no audio device, no I/O):

```rust
use tono_core::dsl::SoundDoc;
use tono_core::render;

let doc: SoundDoc = serde_json::from_str(r#"{
    "name": "blip", "duration": 0.3, "engine": 2,
    "root": { "type": "mul", "inputs": [
        { "type": "sine", "freq": 880 },
        { "type": "env", "a": 0.002, "d": 0.08, "s": 0.0, "r": 0.05 } ] }
}"#)?;

let samples: Vec<f32> = render::render(&doc); // byte-identical every run
```

To *hear* it while prototyping, `tono-play` opens the default speaker:

```rust
tono_play::play_doc(&doc, 0.4)?; // blocking, plays for 0.4 s
```

Play a **factory instrument** live — pitched, polyphonic, with bend / glide /
unison / a brightness sweep:

```rust
use tono_core::instrument::{Instrument, Note};
use tono_core::presets;
use tono_play::{device_sample_rate, Speaker};

let design = presets::preset("warm_lead").unwrap(); // 8 presets built in
let inst = Instrument::new(design, device_sample_rate()?)?;
let speaker = Speaker::open(inst)?;                 // plays until dropped
speaker.control(|i| i.note_on(Note::C4, 0.9));      // drive it live
```

From **Python** (`pip install maturin && make python`):

```python
import json, tono

doc = json.dumps({"name": "blip", "duration": 0.3, "engine": 2,
    "root": {"type": "mul", "inputs": [
        {"type": "sine", "freq": 880},
        {"type": "env", "a": 0.002, "d": 0.08, "s": 0.0, "r": 0.05}]}})

samples = tono.render(doc) # list[float], deterministic
tono.play(doc, 0.4)        # hear it
```

## Quickstart: the MCP server

```sh
curl -fsSL https://raw.githubusercontent.com/marmikshah/tono/master/install.sh | sh
```

The installer sets tono up as stdio (your MCP client spawns it) or as a
shared background HTTP daemon, and prints the matching registration line —
e.g. `claude mcp add --scope user tono -- tono`. Re-run it to update,
or append `-s -- uninstall` to remove.

Prebuilt binaries cover macOS (Apple Silicon), Linux x86_64 and Windows
(grab the `.zip` from [releases](https://github.com/marmikshah/tono/releases/latest));
anything else builds from source with `cargo install --path .`.

Restart your session (MCP tools load at session start), then ask your agent
for sound — *"make me a punchy laser zap"*, *"write a 30-second battle
theme"*. The agent drives the loop:

```
author_sound → analyze (look!) → set_param / edit_sound → export
```

Sounds live under `~/.tono/sounds` (override with `TONO_WORKDIR` —
point it at your game's assets folder to drop renders straight in).

### Server modes

```sh
tono                        # stdio MCP server (default — the client spawns it)
tono --http 127.0.0.1:8787  # streamable HTTP at /mcp
make daemon                     # background HTTP server via launchd / systemd --user
```

### Real instruments

The synth instruments need nothing. For sampled ones, download any free
General MIDI SoundFont once (FluidR3 GM, GeneralUser GS) and point the seq at
it: `wave: "sampler", sf2: "/path/to/gm.sf2", sf2_preset: 0` (0 grand piano,
32 bass, 48 strings; `sf2_bank: 128` = the GM drum map).

## Sessions: deterministic, replayable music

Every mutating tool call is journaled, so a piece of music is a **session
file** — JSON that replays identically every time:

```sh
tono replay docs/examples/band-demo.json --workdir /tmp/tono-demo
```

Eleven annotated examples live in [docs/examples/](docs/examples/), every one
replayed in CI. The deep cuts: the canonical SFX workflow (laser → variants →
bank), a four-instrument band on the mixing console, the complete *River
Flows in You* on the piano instrument (800 notes converted from MIDI with
rubato and sustain pedal intact —
[docs/examples/midi_to_seq.py](docs/examples/midi_to_seq.py) converts any
MIDI), and three loop-ready game BGM tracks — a soft evening theme, a
driving boss battle (kick-ducked bass riff, phrygian sawtooth lead), and a
swung idle-platformer bounce.

And an **iconic-sounds pack** — recognizable classics rebuilt from scratch,
with playable renders in [docs/examples/audio/](docs/examples/audio/):

| Recipe | Play | The trick |
|---|---|---|
| [retro-coin](docs/examples/retro-coin.json) | [▶ mp4](docs/examples/audio/retro-coin.mp4) | B5 grace note into a held E6 — the interval *is* the sound |
| [jump-8bit](docs/examples/jump-8bit.json) | [▶ mp4](docs/examples/audio/jump-8bit.mp4) | exponential square sweep, gone at sustain 0 |
| [waka](docs/examples/waka.json) | [▶ mp4](docs/examples/audio/waka.mp4) | per-note pitch slides alternating up/down — the chomp drawn into the note list |
| [nokia-tune](docs/examples/nokia-tune.json) | [▶ mp4](docs/examples/audio/nokia-tune.mp4) | 13 notes of Gran Vals on the Karplus-Strong pluck |
| [deep-note](docs/examples/deep-note.json) | [▶ mp4](docs/examples/audio/deep-note.mp4) | 8 supersaw mixer tracks gliding from a scattered cluster onto a five-octave D chord |

The ▶ links play right on GitHub (each mp4 is the sound's spectrogram with
the audio as its track — click, press play). OGGs sit next to the mp4s for
direct use.

## Works with atelier

tono is the audio half of a pair:
[**atelier**](https://github.com/marmikshah/atelier) is the same idea for
pixel art — a headless Aseprite-as-API over MCP. Side by side, one agent
session produces a game's art *and* audio: atelier draws the sprites, tiles
and animations; tono scores the SFX, ambience and music; both export
engine-ready packs with manifests, and both record replayable recipes.

## More

- [docs/TOOLS.md](docs/TOOLS.md) — the complete MCP tool reference.
- [docs/cookbook.md](docs/cookbook.md) — the DSL, the instrument table, and
  worked recipes (also served to agents as the `tono://cookbook` resource;
  every example in it is validated by the test suite).
- `make help` lists every target — `make verify` mirrors CI (fmt + clippy +
  test); `make desktop` / `make play` / `make python` build the native faces.

## License

[MIT](LICENSE) — permissive, no warranty.
