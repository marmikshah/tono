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
- For music and layered SFX, a **`tracks` root is the mixing console**: each
  track is a **layer** with a stable slug id (`kick`, `crack`, `tail`),
  pan/gain, an `at` start offset, and `mute`, summed onto a stereo bus through
  a master processor chain (decorrelated reverb tails). Layers are addressed
  by id, never by index, so addresses survive re-arrangement; every layer has
  its own deterministic RNG stream, so editing one never re-grains another.
  Instruments live in the **`seq`** node — see
  [the cookbook](cookbook.md) for the full DSL and instrument table
  (piano, e-piano, organ, strings, bass, GM drum kit, cowbell, pluck, FM,
  and the SoundFont **sampler** for real recorded instruments).

## Authoring

- `author_sound { graph, name? }` — validate → render → analyze in one call.
  Returns a text summary, the spectrogram and waveform images inline, and
  structured `{ id, wav_path, analysis }`. The primary tool.
- `scaffold_layered_sfx { base_freq?, seed?, name? }` — generate a blank,
  band-disciplined 4-layer SFX document (sub / body / top / transient), each a
  mixer layer with a stable id, a band-splitting filter, a one-shot envelope,
  and a starting gain. The sources are neutral placeholders to swap out — a
  correct multi-layer starting point, not a preset.
- `refine_sound { id, graph }` — replace a sound's graph and re-render
  (pushes an undo revision).
- `set_param { id, layer?, path, value }` — change ONE parameter or node by
  JSON path and re-render. With `layer`, the path is relative to that layer's
  node (`env.a`, `notes[3].pitch`); without it, absolute
  (`root.inputs[0].freq`). `value` is a number, a modulator object, or a whole
  node.
- `edit_sound { id, layer?, ops }` — many ordered ops in one re-render:
  `{op:"set", path, value}` · `{op:"insert", path, index?, node}` ·
  `{op:"remove", path, index?}`. Illegal edits fail with the op index and
  reason; the graph is never corrupted.

## Layers (compositional authoring)

The flow: `author_sound` creates the sound with its **first** layer's graph;
`layer { op: "add" }` stacks every next one. One layer per thing you'd fade, pan,
time-shift, or analyze separately — an instrument in a song; the
crack/body/tail of an SFX. Use `mix` only for sub-signals that share one
envelope or filter. Every render reports each layer's post-fader, pre-master
contribution (peak / RMS / energy share), so balance problems name the layer
to fix.

- `layer { id, op, layer, node?, gain?, pan?, at?, mute?, new_id? }` — one tool
  over a sound's mixer layers, addressed by stable id:
  - `op: "add"` — stack a new instrument layer (requires `node`). The first add
    on a plain sound wraps its existing graph as a level-compensated layer named
    after the sound (announced in the response).
  - `op: "set"` — mixer moves (`gain`/`pan`/`at`/`mute`) without touching the
    layer's graph. `mute` is rendered state: exports ship without muted layers.
  - `op: "remove"` — delete a layer (a mixer keeps at least one).
  - `op: "duplicate"` — copy a layer into `new_id`; re-grains noise
    deterministically from the new id (a built-in variation).

## Inspection

- `describe_sound { id }` — the addressing map: every node's editable path,
  type, and parameters. Mixer sounds get per-layer tables (copy the layer id +
  layer-relative path straight into `set_param`), rows for every seq note, and
  the master chain at `root.master[i]`. Call before `set_param` / `edit_sound`.
- `get_sound { id }` — graph + paths + analysis.
- `list_sounds {}` — the library inventory.
- `analyze { id }` — re-run analysis: peak / true-peak / RMS / crest, ≈LUFS,
  spectral centroid / flatness / inharmonicity, attack time + slope, decay /
  onset / silence times, plus both images (log-frequency spectrogram).
- `compare_sounds { a, b }` — metric deltas (b−a) and a 0..1 similarity score;
  converge a sound toward a reference by driving the deltas to zero.
- `review_sound { id, archetype? }` — grade a sound against its archetype's
  targets (attack/centroid/crest/duration) and the universal ship checklist
  (clipping, true-peak, head/tail silence, onset count, loop seam). Returns
  PASS/WARN/FAIL findings, each with the measured value, the target, and the
  concrete fix — a reproducible critique that drives an iterative polish loop
  (see the `sound-review-loop` skill). Omit `archetype` for the universal
  checks only.

## History

- `history { id, op? }` — per-sound revision history (100-deep; every
  refine/set_param/edit/make_loop pushes one; survives restarts).
  `op: "status"` (default) reports `{ undo_depth, redo_depth }`; `op: "undo"`
  reverts to the previous graph (the undone state moves to the redo stack);
  `op: "redo"` re-applies the last undone edit.

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

- `bank { op, name?, bank_id?, sound_id?, category?, rr_group? }` — engine-facing
  packs with categories and round-robin groups. `op: "create"` makes a named
  bank (`name`); `op: "add"` adds/updates a sound's membership (`bank_id`,
  `sound_id`, optional `category` / `rr_group`); `op: "list"` returns every bank.
- `export { id, format, bit_depth?, sample_rate?, dest?, target_lufs?,
  quality? }` — one game-ready file: WAV (8/16-bit, loop chunk), FLAC, or OGG
  Vorbis; optional loudness target without touching the stored graph.
- `export_pack { bank_id?, dest, by_category?, target_lufs?, format?, quality?,
  engine? }` — a pack plus a `sounds.json` manifest. With `bank_id`, every
  member of that bank; omit `bank_id` for the whole library. `engine:
  "godot" | "unity" | "bevy"` also emits `.import` sidecars, `.meta` sidecars
  (stable GUIDs), or a generated `sonarium_sounds.rs`.
- `export_midi { id, dest? }` — write every `seq` in the sound to a Standard
  MIDI File (one track per seq) so a melody / drum pattern round-trips into a
  DAW. Notes map by `(step, len)` on a 480-PPQ grid; the first seq's `bpm` is
  the global tempo. `dest` defaults to `<id>.mid`.

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
