# Roadmap

tono is a deterministic synthesis-graph studio with four faces on one core:
an MCP server (the agent loop), a browser node patcher, a native desktop app you
can play from a keyboard / MIDI, and a parametric in-engine runtime. What exists
is documented in the README, [docs/TOOLS.md](docs/TOOLS.md), and
[docs/runtime.md](docs/runtime.md); this file is the backlog.

## Near term

- **Track sends** — shared effect buses at per-track send levels (one reverb
  for the whole mix), complementing today's insert chains.
- **Per-note pan** — stereo placement inside one seq (a keyboard's bass-left /
  treble-right image) on top of the per-track pan.
- **More instruments** — brass/lead (filtered saw + vibrato), choir/pad,
  mallets family presets; the per-note voice architecture makes each ~30 lines
  plus a behavioral test.
- **SF3 support** — compressed SoundFonts (FluidR3 ships as 15 MB sf3 vs
  141 MB sf2).

## The feedback loop

- **Loudness graph** — short-term LUFS over time as a third feedback image, so
  an agent reads an arrangement's dynamics at a glance.
- **Section analysis** — `analyze { from_secs, to_secs }` windows, for
  judging a drop against a break without exporting slices.
- **Chromagram / key detection** — "what key is this in?" as numbers, for
  remix work against references.

## Production

- **Mastering presets** — opinionated master-bus starting points (streaming
  loudness, game SFX, club) expressed as ordinary processor chains.
- **Tempo map in `seq`** — native ritardando/accelerando instead of the
  fine-grid workaround the MIDI converter uses today.
- **MIDI import** — `import_midi` to bring a `.mid` straight into a `seq`
  (export already round-trips out via `export_midi`).

## Ecosystem

- **Joint workflows with [atelier](https://github.com/marmikshah/atelier)** —
  one agent session producing a game's art *and* audio: shared asset manifest
  conventions, a combined "ship a game jam pack" recipe.
- **Wwise/FMOD manifest flavours** — beyond the Godot/Unity/Bevy emitters.
- **Game-engine runtime bindings** — the parametric
  [patch](docs/runtime.md) runs in-engine today via the pure core; first-class
  Bevy / Godot / Unity bindings (an asset type + a one-call render) are next.
