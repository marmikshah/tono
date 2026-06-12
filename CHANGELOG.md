# Changelog

## 1.1.0 — 2026-06-12

Compositional authoring: a sound is now a document you build up in named,
addressable layers, each rendered on its own deterministic stream. Backward
compatible — v1 documents omit the version field, keep their original render
semantics, and replay byte-for-byte.

### Layered authoring
- **Stable layer ids**: every track carries a unique, validated slug id, an
  `at` start offset (applied post-render, so RNG consumption never depends on
  placement), and persisted `mute`. Ids are backfilled deterministically at the
  build chokepoint, so replays mint the same ids.
- **Schema v2 per-layer RNG streams**: each track and the master bus gets its
  own deterministic noise stream keyed by layer id — adding, removing, or muting
  one layer never re-grains a sibling. v1 docs keep the threaded stream.
- New tools — **`add_layer`** (the compositional flow: the first call wraps a
  plain root as a level-compensated layer named after the sound; duplicates
  rejected with the layer listing), **`set_layer`** (mixer fields), **`layer_ops`**
  (remove/duplicate). `set_param` / `edit_sound` take a `layer` arg with
  node-relative paths; `describe_sound` emits per-layer tables with
  ready-to-paste layer-relative paths and a row for every seq note.
- **Per-layer contribution stats**: each render captures every layer's
  post-fader pre-master peak / RMS / energy share from the same pass and prints
  a compact per-layer balance line (muted layers flagged); the stats persist on
  `Analysis`. `morph_sounds` unifies layer identity positionally, so
  independently-minted ids no longer block morphs between same-shaped documents.

### Performance & history
- **Single-pass render**: mixer documents were fully rendered twice per
  build/export (stereo for the WAV, mid for analysis). `render_product()` now
  yields both from one pass; build/export/pack/rehydrate and `make_loop` reuse it.
- Undo history deepens **20 → 100** — compositional editing burns revisions fast
  and graphs are small JSON.

### Fixes
- Mutating tools now build **before** checkpointing, so a rejected graph leaves
  history, redo, and the journal untouched (a failed call used to push a no-op
  revision, wipe redo, and desync replay).
- Replay no longer stamps the current schema version onto version-less journaled
  steps; `rehydrate` backfills track ids and per-layer stats so pre-layering
  mixer docs survive a restart; `humanize` trims the master chain on Tracks roots
  instead of wrapping the root (which validation rejected on every multi-track
  sound). Closes 18 issues from an adversarial branch review.

### Skill & showcases
- Ship a **sound-designer** project skill: the listen-and-fix loop, how to read
  every analysis metric and both feedback images, per-archetype numeric targets,
  symptom-to-fix recipes, the layered workflow, and the ship checklist.
- Three loop-ready game-BGM showcases composed on the console with it —
  **evening-glade** (soft BGM), **iron-gauntlet** (boss battle), **sunny-steps**
  (idle platformer) — replace the phonk remix; both River showcases and the
  retro-coin / jump-8bit SFX got a polish pass. Eleven examples, all replayed in
  CI with playable renders.

## 1.0.0 — 2026-06-07

First release. A headless sound studio for AI agents, driven over MCP.

### Instruments & synthesis
- Polyphonic sequencer (`seq`) with a core instrument set: **piano** (detuned
  string pair, velocity brightness, pitch-dependent decay), **e-piano**
  (Rhodes tine), **organ** (tonewheel drawbars + percussion), **strings**
  (ensemble swell), **bass** (filtered + sub), **kit** (full drum kit on the
  General MIDI map), pitched **cowbell**, **pluck** (Karplus-Strong), tunable
  **fm** mallets/bells — plus raw band-limited square/saw/triangle, sine, FM,
  supersaw and three noise colours.
- **`sampler`**: real recorded instruments from any SoundFont (`sf2` path +
  GM program; bank 128 = drum map), rendered deterministically via rustysynth.
- Note-name pitches (`"C4"`, `"midi:60"`), per-parameter modulators
  (`slide`/`lfo`/`arp`/`env`), `swing` + `humanize` groove.

### Production
- **`tracks` mixing console**: per-track equal-power pan and fader onto a true
  stereo bus, master processor chain, decorrelated (Freeverb-spread) reverb
  tails; sampler tracks keep their native stereo.
- Effects: filters + EQ, drive, ringmod, chorus/flanger/phaser, compressor,
  **`duck` sidechain pumping**, bitcrush/downsample, delay, reverb.
- Output stage: LUFS-targeted soft-knee limiting to a true-peak ceiling;
  seamless loops (equal-power crossfade + WAV `smpl` chunk); WAV/FLAC/OGG.

### The agent loop
- Every render returns analysis (peak/true-peak/RMS/crest, ≈LUFS, spectral
  centroid, transients) plus **spectrogram and waveform images**;
  `compare_sounds` reports deltas + similarity.
- Surgical editing by JSON path (`describe_sound` → `set_param` /
  `edit_sound`), 20-deep undo/redo, persistent slug-id library.
- Variations on agent-made sounds: `mutate_sound`, `generate_variants`,
  `humanize`, `morph_sounds`.
- Banks → `sounds.json` manifests + engine files (Godot/Unity/Bevy).

### Sessions
- Every mutating call journaled; `save_session` / `replay_session` (and the
  `sonarium replay` CLI) reproduce a project **byte-for-byte** in a fresh
  directory. Annotated recipe files double as tutorials; nine showcases —
  including the complete *River Flows in You*, its phonk remix, and an
  iconic-sounds pack — replay in CI, with playable renders committed.

### Ops
- stdio + streamable-HTTP transports; self-managing launchd/systemd daemon;
  one-line installer; tagged binary releases (macOS arm64, Linux x86_64,
  Windows); dual-licensed MIT/Apache-2.0.
