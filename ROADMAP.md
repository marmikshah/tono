# Roadmap

sonarium is a sound-engineering studio driven over MCP: a deterministic
synthesis-graph DSL, a core instrument set plus a SoundFont sampler, a stereo
mixing console, analysis feedback (numbers + spectrogram/waveform images), and
replayable session files. What exists is documented in the README and
[docs/TOOLS.md](docs/TOOLS.md); this file is the backlog.

## Near term

- **Track sends** — shared effect buses at per-track send levels (one reverb
  for the whole mix), complementing today's insert chains.
- **Automation lanes** — parameter changes over song time at the track level
  (volume rides, filter sweeps across sections); per-sample modulators already
  cover the node level.
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
- **MIDI export** — `export_midi { id }` for round-tripping compositions into
  DAWs.

## Ecosystem

- **Joint workflows with [atelier](https://github.com/marmikshah/atelier)** —
  one agent session producing a game's art *and* audio: shared asset manifest
  conventions, a combined "ship a game jam pack" recipe.
- **Wwise/FMOD manifest flavours** — beyond the Godot/Unity/Bevy emitters.
