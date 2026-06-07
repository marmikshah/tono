# MCP tool reference

The complete tool surface, as advertised to MCP clients.

## The model

- A **sound** is the unit: one `SoundDoc` — metadata (duration, sample rate,
  seed, stereo, playback, normalize) plus a single **root node** (the synthesis
  graph). Rendering is a pure function of `(graph, seed, sample_rate)`: the
  same document always produces byte-identical audio. Sounds are stored under
  `~/.sonarium/sounds/` (override with `SONARIUM_WORKDIR`) as
  `<id>.wav` + `<id>.json` (the graph — the source of truth) and are addressed
  by a stable slug **id** derived from the name (`laser_zap`), usable directly
  as an engine asset key. The library survives restarts: graphs are re-rendered
  on startup.
- The loop: `author_sound` → read the analysis, view the spectrogram +
  waveform → `set_param` / `edit_sound` (surgical) or `refine_sound` (whole
  graph) → `export`.
- A **session** is the ordered journal of every mutating tool call
  (`session.jsonl`). `save_session` snapshots it; `replay_session` (or the
  `sonarium replay` CLI) reproduces the whole project byte-for-byte in a fresh
  working directory.
- For music, a **`tracks` root is the mixing console**: per-track pan/gain
  onto a stereo bus, master processor chain, decorrelated reverb tails.
  Instruments live in the **`seq`** node — see
  [the cookbook](cookbook.md) for the full DSL and instrument table
  (piano, e-piano, organ, strings, bass, GM drum kit, cowbell, pluck, FM,
  and the SoundFont **sampler** for real recorded instruments).

## Authoring

- `author_sound { graph, name? }` — validate → render → analyze in one call.
  Returns a text summary, the spectrogram and waveform images inline, and
  structured `{ id, wav_path, analysis }`. The primary tool.
- `refine_sound { id, graph }` — replace a sound's graph and re-render
  (pushes an undo revision).
- `set_param { id, path, value }` — change ONE parameter or node by JSON path
  (`root.inputs[0].freq`, `root.tracks[1].node.notes[3].gain`) and re-render.
  `value` is a number, a modulator object, or a whole node.
- `edit_sound { id, ops }` — many ordered ops in one re-render:
  `{op:"set", path, value}` · `{op:"insert", path, index?, node}` ·
  `{op:"remove", path, index?}`. Illegal edits fail with the op index and
  reason; the graph is never corrupted.

## Inspection

- `describe_sound { id }` — the addressing map: every node's editable path,
  type, and parameters. Call before `set_param` / `edit_sound`.
- `get_sound { id }` — graph + paths + analysis.
- `list_sounds {}` — the library inventory.
- `analyze { id }` — re-run analysis: peak / true-peak / RMS / crest, ≈LUFS,
  spectral centroid, attack/decay/onset/silence times, plus both images.
- `compare_sounds { a, b }` — metric deltas (b−a) and a 0..1 similarity score;
  converge a sound toward a reference by driving the deltas to zero.

## History

- `undo_sound { id }` / `redo_sound { id }` — step through a 20-deep per-sound
  revision history (every refine/set_param/edit/make_loop pushes one). Survives
  restarts.
- `history { id }` — `{ undo_depth, redo_depth }`.

## Variations (on sounds the agent already made)

- `mutate_sound { id, amount?, seed? }` — jitter numeric parameters by up to
  `amount` (clamped valid) into a new sound.
- `generate_variants { id, count, amount?, seed?, target_lufs? }` — N
  round-robin takes, each perturbed and loudness-matched.
- `humanize { id, count?, pitch_cents?, gain_db?, seed? }` — performer-style
  takes: ONE coherent pitch shift + level trim each; identity untouched.
- `morph_sounds { a, b, steps? }` — in-betweens of two same-shaped graphs
  (charge tiers, damage levels). Numbers lerp; note names lerp in Hz.

## Loops

- `make_loop { id, crossfade_secs?, start_secs?, end_secs? }` — equal-power
  crossfade the region's tail onto its head: a seamless loop body. Reports the
  loop-seam discontinuity in dB; the WAV carries a `smpl` chunk engines read.

## Banks & export

- `create_bank { name }` / `add_to_bank { bank_id, sound_id, category?,
  rr_group? }` / `list_banks {}` — engine-facing packs with categories and
  round-robin groups.
- `export { id, format, bit_depth?, sample_rate?, dest?, target_lufs?,
  quality? }` — one game-ready file: WAV (8/16-bit, loop chunk), FLAC, or OGG
  Vorbis; optional loudness target without touching the stored graph.
- `export_bank { bank_id, dest, by_category?, target_lufs?, format?, quality?,
  engine? }` — every member plus a `sounds.json` manifest; `engine:
  "godot" | "unity" | "bevy"` also emits `.import` sidecars, `.meta` sidecars
  (stable GUIDs), or a generated `sonarium_sounds.rs`.
- `export_all { dest, ... }` — the whole library, same options.

## Sessions

- `save_session { dest? }` — snapshot the journal to a portable session file.
- `replay_session { path }` — re-apply a saved session (raw journal or
  annotated recipe) into a **fresh** session; refuses a non-empty working
  directory because ids derive from names. Same calls, same seeds,
  byte-identical audio. CLI equivalent:
  `sonarium replay FILE [--workdir DIR]`.

## Resources

- `sonarium://schema/sounddoc` — the `SoundDoc` JSON Schema
  (`application/json`).
- `sonarium://cookbook` — example graphs, instrument table, and authoring tips
  (`text/markdown`, single-sourced from [docs/cookbook.md](cookbook.md)).
