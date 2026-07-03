# Changelog

## 1.5.0 — 2026-07-03

Per-track mixing on the song builder. Instruments gain `.reverb(0..1)` (a reverb
send that wraps the track in a reverb), and `.swing(0..1)` / `.humanize(0..1)`
to override the song-global groove per track. Byte-safe — a dry, unset track is
identical to before.

## 1.4.0 — 2026-07-03

A composition system on the one deterministic core: a catalog of ready-to-play
synthesized instruments and a fluent multi-instrument song builder, plus a deep
pass on the instrument voices. Pure synthesis — no soundfonts, no files — and
every change is byte-safe: existing documents render bit-for-bit as before.

### Instrument catalog + song builder
- **`tono_core::catalog`** — ready-to-play instruments (grand piano, electric
  piano, organ, strings, bass, guitar, drums) with variants, each a tuned voice
  you hand to the song builder.
- **`Song::add(instrument, |t| …)`** — a fluent, beat-timeline builder: place
  notes with `.at(beat).note/.chord`, step a melody with `.play/.rest`, hit drums
  with `.kick/.snare/.hat`. Compiles to the deterministic `tracks` SoundDoc.
- `cargo run -p tono-play --example lofi` / `band` — full songs in a few lines.

### Deeper voices (all byte-safe, opt-in per variant)
- **Grand piano** — an inharmonic additive model (stretched partials, per-partial
  decay, hammer-strike spectrum, detuned unison), gated at `engine` 3; six
  variants (bright/mellow/felt/upright/honky-tonk) via five tone knobs.
- **Drums** — four synthesized kits (classic/acoustic/electronic/808).
- **Bass** — finger/pick/sub/synth via ten tone knobs.
- **Guitar** — nylon/steel/electric via body resonance, pick noise, and tone.

## 1.3.0 — 2026-07-01

The engine becomes a **library + CLI**. The MCP server is removed entirely;
`tono-core` is the published deterministic engine and `tono render` is the CLI
that turns a `SoundDoc` into audio plus analysis images. Install via Cargo
(`cargo add tono-core` / `cargo install tono`).

## 1.2.0 — 2026-06-28

Higher-fidelity synthesis gated so it never breaks byte-stability, a workspace
split, and a leap from "headless engine" to a **studio you can design *and*
play sound in** — a browser playground and an optional native desktop app, both
on the one deterministic core, with the MCP face unchanged.

### Real-time engine + native desktop studio
- **`tono-core::stream::Player`** — the host-agnostic audition seam an audio
  callback fills in blocks. The invariant that makes live editing safe is pinned
  by test: audio served block-by-block is **byte-identical to an offline bounce**
  of the same document.
- **Playable synth** — a gated streaming `voice` (band-limited oscillator + ADSR
  with gate-on/off, reusing the renderer's exact kernels) and a `PolySynth`
  voice allocator with voice-stealing. The live-performance path, distinct from
  the byte-identical offline render.
- **`tono-desktop`** — an **optional** Tauri + `cpal` native studio running
  the full node patcher with real-time audio: edits play live, ▶ Play auditions
  the patch, and you **play the patch like an instrument** from the computer
  keyboard (A–K) or a hardware **MIDI** controller (native CoreMIDI via `midir`),
  mixed with the preview. Kept out of the default build / CI (heavy webview/cpal
  deps); built via `make desktop`.

### Manual studio editors (one frontend, web + desktop)
- **Node patcher** picks its backend at boot — WASM + Web Audio in a browser, or
  native `cpal` + the core via Tauri commands on the desktop — so one frontend
  serves both.
- **Piano roll** for `seq` nodes (draw notes, length, bpm/steps-per-beat).
- **Channel-strip mixer** for `tracks` documents — vertical faders, pan, mute,
  **solo** (transient: heard, not saved), and **live per-layer meters** from the
  render's per-layer stats; a master strip with the bus meter + LUFS.
- **Inline modal-modes table** (freq/decay/gain per partial) — closes the last
  "edit in JSON" gap in the patcher.

### Track automation
- **`Track.automation`** — gain/pan lanes of `{t, v}` breakpoints over song time
  (volume rides, pan moves), linearly interpolated. A track with no automation
  stays on the constant fast path, so every existing document renders
  **byte-identically**; tests pin a constant-lane-equals-static invariant and a
  ramp that provably fades. Settable by an agent through the existing graph tools
  (`set_param` / `edit_sound` / `refine_sound`), and drawn in a lane editor in
  the playground mixer.

### Interop
- **`export_midi { id, dest? }`** — write every `seq` to a Standard MIDI File
  (one track per seq) so a melody / drum pattern round-trips into a DAW.

### Repo standards
- Engineering-standards pass: `LICENSE` (dual MIT/Apache), `.editorconfig`,
  `CLAUDE.md`, `.env.example`, a `pre-commit` hook, and the canonical Makefile
  targets; default branch is `master`; the committed WASM is built with
  `--remap-path-prefix` so it carries no build-machine paths.

### Tool surface consolidation (30 → 23)
- **Op-based merges** for the admin clusters, so the agent picks from a smaller,
  cleaner surface: `history { id, op: status|undo|redo }` (was undo_sound /
  redo_sound / history); `layer { id, op: add|set|remove|duplicate, … }` (was
  add_layer / set_layer / layer_ops); `bank { op: create|add|list, … }` (was
  create_bank / add_to_bank / list_banks); `export_pack { bank_id?, … }` (was
  export_bank / export_all — omit `bank_id` for the whole library). The hot
  authoring loop (author_sound, set_param, edit_sound, analyze, review_sound,
  …) is untouched, and `export` (single sound) stays its own tool.
- **Replay is unaffected.** Each merged op still journals under its original
  name, so every saved session and shipped recipe replays byte-for-byte.

### Workspace + browser playground
- **`tono-core` crate** — the pure, headless engine (graph DSL, DSP,
  deterministic renderer, analysis, critique, graph transforms) extracted into
  its own crate with **no I/O, no MCP, no transport**. The `tono` binary is
  now a thin shell (MCP server, encoders, persistence, daemon) that re-exports
  it, so every existing path is unchanged. One core, three targets: native MCP,
  WASM, and a future in-engine runtime.
- **WASM build + manual node patcher** — `tono-wasm` compiles the core to
  WebAssembly; `make wasm` emits it into `docs/playground/`, a zero-install
  browser studio where a human **builds a sound effect by hand, modular-synth
  style**: drop nodes from a palette (oscillators, envelopes, filters,
  mix/mul…), drag them anywhere, **wire output ports to input ports manually**,
  and tweak each node's parameters inline (sliders / dropdowns / modulator
  pickers) — everything flowing into an `OUT ▶` terminal. Multi-track sounds
  work too: a `mixer` node sums `layer` nodes (each with pan / gain / start
  offset / mute), and the serial processors between the mixer and `OUT` become
  the master chain — i.e. a `tracks` document. The patch serializes to a
  `SoundDoc` (serial effect runs auto-fold into a `chain`) and renders live
  to audio plus the same spectrogram / waveform / analysis an agent sees,
  **byte-identically to the native engine**; a two-way JSON drawer exposes the
  exact document an agent edits. The SoundFont sampler voice is the only one
  unavailable in the browser.
- **In-memory analysis** — `analysis::stats` (numbers, no filesystem) and
  `spectrogram_png` / `waveform_png` (PNG bytes) split out of the disk-writing
  `analyze`, so a render can hand back feedback without a disk round-trip.

### Engine revisions
- **New `engine` document field** — a DSP-kernel revision number, independent
  of the schema `version`. Omitted ⇒ engine 0 (the original kernels): every
  existing document and session replays **byte-for-byte**. New documents are
  stamped with the current `ENGINE_VERSION`; `refine_sound` preserves a sound's
  existing engine. This is what lets a fidelity upgrade ship without altering
  older renders.

### Anti-aliased distortion (engine 1)
- **`drive` now uses antiderivative anti-aliasing (ADAA)** on engine-1
  documents — the `hard` and `fold` shapers no longer spray inharmonic
  foldback across the spectrum. First-order ADAA with a one-pole DC blocker;
  per-node `"aa": false` opts back into the raw aliasing curve. Legacy
  (engine-0) documents are unaffected and stay bit-exact.

### Physical impacts (new nodes)
- **`modal`** — a resonator bank: N parallel damped sinusoidal partials
  (`modes: [{freq, decay, gain}]`) excited by the incoming chain signal. Bells,
  glass, metal, wood, ceramic, coins, and the resonant body of UI/impact
  sounds — none of which the oscillators voice cleanly. Each mode is a
  normalised two-pole resonator (impulse-response peak ∝ `gain`, decay exact),
  so the bank is cheap, stable, and fully deterministic. Modes are individually
  addressable (`…modes[i].freq`).
- **`impact`** — a strike exciter: a single unit-area force pulse whose
  `hardness` shapes its brightness (which modes light up) and `velocity` its
  energy. The exciter half of the `chain[ impact, modal ]` struck-body pair.
- New example **`docs/examples/struck-bell.json`** (a struck bell + a coin
  ding), replayed in CI like every other recipe.

### Texture & environmental synthesis (new primitives)
- **`dust`** — a sparse stochastic source: a Poisson click train (`density`
  events/sec, each decaying over `decay` seconds; 0 = bare impulses), smoothed
  so overlapping grains sum. The generator behind fire crackle, rain, geiger
  ticks, sparks, and debris. Draws from the layer's deterministic stream like
  `noise`.
- **`rand`** — a random-walk modulator: smooth, NON-periodic drift between
  `from` and `to` at `rate` targets/sec. The organic motion the periodic
  modulators lack — wind gusting, fire flicker, drifting detune. Seeded only
  from its own fields (with an optional `seed` to decorrelate), so it is
  deterministic and stable under sibling edits.
- New example **`docs/examples/fire-and-wind.json`** — a looped campfire
  (`dust` crackle + `rand`-driven roar) and gusting wind (two decorrelated
  `rand` walks), replayed in CI.

### Review loop
- **New `review_sound { id, archetype? }` tool** — a deterministic critique
  engine. Grades a sound against its archetype targets (laser / coin / jump /
  impact / ui / ambience / bgm) and the universal ship checklist (clipping,
  true-peak, head/tail silence, onset count, loop seam), returning PASS / WARN /
  FAIL findings each with the measured value, the target, and the concrete fix.
  Reproducible — a given sound always reviews the same way. Read-only.
- **New `sound-review-loop` skill** — drives Review → Polish → Review:
  `review_sound` → apply the top finding's fix with one `set_param` → re-review
  → `undo_sound` on a regression → repeat until PASS. The user can supply review
  in their own words at any iteration and it takes over.

### Craft tooling
- **New `scaffold_layered_sfx { base_freq?, seed?, name? }` tool** — generates a
  blank, band-disciplined 4-layer SFX document (sub / body / top / transient),
  each a mixer layer with a stable id, a band-splitting filter, a one-shot
  envelope, and a starting gain. Sources are neutral placeholders the agent
  swaps out: a correct multi-layer *structure*, not a preset. Stamped schema v2
  (independent per-layer noise) + the current engine; journaled and replayable.
  New CI-replayed example `docs/examples/layered-sfx-scaffold.json`.

### Analyzer (sharper ears)
- **Log-frequency spectrogram** — the feedback image's frequency axis is now
  logarithmic, so bass/low-mids and modal partials are legible instead of
  crushed into the bottom strip. Image-only; audio bytes are unchanged.
- **New metrics on every render**: `spectral_flatness` (tonal vs. noisy),
  `inharmonicity` (off-harmonic-grid energy — also an aliasing/foldback
  indicator), and `attack_slope_db_per_ms` (transient sharpness). All are
  reporting-only — they never feed the render's loudness/limiting stage, so
  determinism is untouched.

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
  `tono replay` CLI) reproduce a project **byte-for-byte** in a fresh
  directory. Annotated recipe files double as tutorials; nine showcases —
  including the complete *River Flows in You*, its phonk remix, and an
  iconic-sounds pack — replay in CI, with playable renders committed.

### Ops
- stdio + streamable-HTTP transports; self-managing launchd/systemd daemon;
  one-line installer; tagged binary releases (macOS arm64, Linux x86_64,
  Windows); dual-licensed MIT/Apache-2.0.
