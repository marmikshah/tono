<h1 align="center">Sonarium</h1>

<p align="center"><em>An MCP server that turns an AI agent into a sound engineer.</em></p>

Sonarium is an **orchestrator, not a generator** — think GarageBand for
agents. It provides **instruments** — a SoundFont **sampler** for real recorded
instruments (point it at any free GM bank), plus synthesized piano, e-piano,
organ, strings, bass, a GM-mapped drum kit, pitched cowbell, plucked string,
and FM mallets in a polyphonic sequencer with swing/humanize groove, **effects** (filters, EQ, drive, mod
fx, dynamics, delay, reverb), and mixing / mastering tools — and an agent
composes with them through MCP tool calls. There is no canned content; the
agent does the sound design and arranging, the server does the DSP.

The agent authors a **symbolic synthesis graph** (oscillators → envelopes →
filters → modulation → mix). Sonarium renders it **deterministically** — the
same graph and seed always produce identical audio — and feeds back analysis:
levels, loudness, spectral centroid, transient descriptors, plus a
**spectrogram and a waveform image**. The agent iterates by inspection, like a
sound designer at a DAW, then exports game-ready WAV / FLAC / OGG.

Pure Rust, local, offline. No GPU, no API keys.

## Quick start

```bash
make daemon     # release build + install & start the background service (launchd / systemd)
# or foreground: make
```

Then point a client at it:

```bash
claude mcp add --transport http sonarium http://127.0.0.1:8787/mcp
```

`make help` lists every target. Common ones: `make serve` (foreground HTTP),
`make stdio`, `make test`, `make check` (fmt + clippy + tests — the pre-commit
gate). Override host / output dir: `make serve BIND=127.0.0.1:9000
WORKDIR=./game/assets/audio`.

### stdio (client spawns the binary)

```bash
claude mcp add sonarium -e SONARIUM_WORKDIR=/path/to/game/assets/sfx -- /path/to/sonarium
```

Claude Desktop (`claude_desktop_config.json`):

```json
{
  "mcpServers": {
    "sonarium": {
      "command": "/path/to/sonarium",
      "env": { "SONARIUM_WORKDIR": "/path/to/game/assets/sfx" }
    }
  }
}
```

Renders and exports land in `SONARIUM_WORKDIR` (default `~/.sonarium/sounds`)
— point it at your game's assets folder to drop sounds straight in.

## What it does

- **Author & surgically edit.** `author_sound` renders a graph;
  `describe_sound` → `set_param` / `edit_sound` change one parameter or node by
  path (`root.inputs[0].freq`) without re-sending the graph; `undo_sound` /
  `redo_sound` step a 20-deep per-sound history. Sounds **persist across
  restarts** under stable slug ids (`laser_zap`) usable directly as engine
  asset keys.
- **Hear with eyes.** Every render returns numeric analysis (peak, true peak,
  RMS, crest, ≈LUFS, spectral centroid, attack/decay/onset/silence times) plus
  a **spectrogram and a waveform image**. `compare_sounds` gives metric deltas
  + a similarity score to converge on a reference.
- **Reproducible sessions.** Every mutating tool call is journaled to
  `session.jsonl`. `save_session` snapshots it; `replay_session` re-applies a
  saved journal into a fresh working directory (enforced) — same calls, same
  seeds, **byte-identical audio**. A session file is a portable, diffable
  project file ([example](examples/laser_session.jsonl), replayed in CI).
- **Variations, not presets.** `mutate_sound` nudges parameters;
  `generate_variants` makes N level-matched round-robin takes; `humanize`
  applies one coherent pitch + level shift per take; `morph_sounds`
  interpolates two same-shaped designs (charge tiers, damage levels).
- **Ship-ready output.** Top-level `normalize { target_lufs, ceiling_dbtp }`
  loudness-matches through a soft-knee true-peak limiter. `playback: loop`
  renders **seamless loops** (equal-power crossfade; WAV carries a `smpl` loop
  chunk). Export WAV / FLAC / OGG Vorbis.
- **Packs, not files.** Banks with categories + round-robin groups;
  `export_bank` / `export_all` write every member plus a `sounds.json`
  manifest, and optionally **engine files**: Godot `.import` sidecars, Unity
  `.meta` (stable GUIDs), or a generated Bevy `sonarium_sounds.rs`.

## Tools

| Tool | Input | Output |
|------|-------|--------|
| `author_sound` | `{ graph, name? }` | summary + spectrogram/waveform images + `{ id, wav_path, analysis }` |
| `refine_sound` | `{ id, graph }` | same — replaces a sound's graph and re-renders |
| `describe_sound` | `{ id }` | every node's editable `path`, `type`, and params |
| `set_param` | `{ id, path, value }` | change one param/node by path and re-render |
| `edit_sound` | `{ id, ops }` | many `set` / `insert` / `remove` ops in one re-render |
| `undo_sound` / `redo_sound` | `{ id }` | step through the 20-deep edit history |
| `history` | `{ id }` | `{ undo_depth, redo_depth }` |
| `get_sound` / `list_sounds` | `{ id }` / `{}` | graph + analysis / inventory |
| `analyze` | `{ id }` | stats + both images |
| `compare_sounds` | `{ a, b }` | metric deltas (b−a) + 0..1 similarity |
| `export` | `{ id, format, bit_depth?, sample_rate?, dest?, target_lufs?, quality? }` | WAV / FLAC / OGG, optional loudness target |
| `mutate_sound` | `{ id, amount?, seed? }` | a perturbed variant |
| `generate_variants` | `{ id, count, amount?, seed?, target_lufs? }` | N level-matched round-robin takes |
| `humanize` | `{ id, count?, pitch_cents?, gain_db?, seed? }` | coherent performer-style takes |
| `morph_sounds` | `{ a, b, steps? }` | in-betweens of two same-shaped graphs |
| `make_loop` | `{ id, crossfade_secs?, start_secs?, end_secs? }` | seamless loop + seam dB |
| `create_bank` / `add_to_bank` / `list_banks` | — | sound packs with categories + rr groups |
| `export_bank` / `export_all` | `{ dest, format?, target_lufs?, engine?, ... }` | files + `sounds.json` + engine files |
| `save_session` | `{ dest? }` | snapshot the session journal |
| `replay_session` | `{ path }` | re-apply a saved session deterministically |

### Resources

- `sonarium://schema/sounddoc` — the `SoundDoc` JSON Schema.
- `sonarium://cookbook` — example graphs and authoring tips
  (single-sourced from [`docs/cookbook.md`](docs/cookbook.md)).

## The synthesis-graph DSL

A sound is one `SoundDoc`:

```json
{ "name": "laser_zap", "duration": 0.22, "sample_rate": 44100, "seed": 0, "root": { ... } }
```

`root` is a single node; every node evaluates to a mono signal. Add optional
top-level `stereo` (wide / Haas) for BGM and ambience, `playback` for seamless
loops, and `normalize` for loudness-matched output.

**Sources** — `square{freq,duty}` (duty modulatable ⇒ PWM), `triangle`,
`sawtooth`, `sine`, `noise{color: white|pink|brown}`, `fm{freq,ratio,index}`,
`super{wave,freq,voices,detune_cents}`, and `seq{bpm,steps_per_beat,wave,duty,
env,notes}` for melodies, basslines, and drum patterns — pitches read musically
(`"C4"`, `"F#3"`, `"midi:60"`).
**Envelope** — `env{a,d,s,r,punch}`.
**Combinators** — `mix` (sum), `mul` (source × envelope), `chain` (source →
processors).
**Processors** — `lowpass`/`highpass`/`bandpass`/`notch{cutoff,q}`,
`peak{cutoff,q,gain_db}`, `lowshelf`/`highshelf{cutoff,gain_db}`, `gain`,
`drive{amount,shape}`, `ringmod`, `chorus`, `flanger`, `phaser`, `compress`,
`bitcrush`, `downsample`, `delay`, `reverb`.
**Modulators** (any numeric param) — `slide`, `lfo`, `arp`, and
`env{a,d,s,r,from,to}` (an ADSR mapped onto a range ⇒ filter / pitch
envelopes).

### Real instruments

Download any free General MIDI SoundFont once (FluidR3 GM, GeneralUser GS),
then `wave: "sampler", sf2: "/path/to/gm.sf2", sf2_preset: 0` plays the notes
on a real recorded grand (32 = bass, 48 = strings; `sf2_bank: 128` = the GM
drum map). Renders stay deterministic. The `duck` processor adds sidechain
pumping, and every seq takes `swing` / `humanize` for groove.

### Example — laser zap

```json
{
  "name": "laser_zap",
  "duration": 0.22,
  "root": {
    "type": "mix",
    "inputs": [
      { "type": "mul", "inputs": [
        { "type": "square", "duty": 0.25,
          "freq": { "slide": { "from": 880, "to": 180, "secs": 0.18, "curve": "exp" } } },
        { "type": "env", "a": 0.0, "d": 0.18, "s": 0.0, "r": 0.02, "punch": 0.3 }
      ]},
      { "type": "mul", "inputs": [
        { "type": "noise" },
        { "type": "env", "a": 0.0, "d": 0.04, "s": 0.0, "r": 0.0 }
      ]}
    ]
  }
}
```

The [cookbook](docs/cookbook.md) has many more — sequenced melodies, FM bells,
filter envelopes, layered impacts, looping ambience beds. Every JSON example in
it is parsed and validated by the test suite.

### Showcase — a real piece of music

[`examples/river_flows_in_you.jsonl`](examples/river_flows_in_you.jsonl) is a
session file that renders the **complete** *River Flows in You* (Yiruma):
800 notes played on the built-in `piano` instrument, with the performance's
tempo map (rubato) and sustain pedal intact, through reverb, stereo width,
and a −14 LUFS master — authored as one `author_sound` call and replayable
with one `replay_session` call.
[`examples/midi_to_seq.py`](examples/midi_to_seq.py) is the converter that
turns any MIDI file into `seq` notes (tempo-map-aware, pedal-aware), so real
scores can drive Sonarium.

[`examples/band_demo.jsonl`](examples/band_demo.jsonl) is the instrument set
on the **mixing console**: drum kit + bass + e-piano + string pad over an
Am–F–C–G groove, each on its own panned track, glued by a master-bus
compressor and stereo-spread reverb — a true stereo production from one
`author_sound` call.

[`examples/river_phonk.jsonl`](examples/river_phonk.jsonl) is the remix
proof: the same River hook re-gridded to a rigid 140 bpm and rebuilt as
phonk — pitched **cowbell** lead, tanh-driven 808 sub, bitcrushed lo-fi
piano, hat rolls, drop/break/drop arrangement, slammed through the bus
compressor to −12 LUFS.

## Build

```bash
make release        # → target/release/sonarium   (or: cargo build --release)
make test           # the full unit + integration suite
make check          # fmt + clippy -D warnings + test  (pre-commit gate)
```

Rust 1.88+ (edition 2024). OGG encoding builds vendored libvorbis, so a C
toolchain is required. CI runs the same fmt + clippy + test gate on pushes to
`main` and on pull requests.

## Production notes

- **Stay on loopback.** The HTTP server is meant for same-machine clients;
  don't expose it to a network.
- **Deterministic by contract.** A sound is fully determined by its graph +
  seed; the graph JSON is written next to each WAV, so renders are
  re-creatable and version-controllable. The PRNG is pinned to reference
  vectors by tests.
- Logs go to stderr (`RUST_LOG=debug` for more); the stdio JSON-RPC stream
  stays clean.

## License

Dual-licensed under [MIT](LICENSE-MIT) or [Apache-2.0](LICENSE-APACHE), at
your option.
