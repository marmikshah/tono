# Changelog

## Unreleased

A full-codebase review hardening pass: the input edges that could render NaN,
hang, panic, or silently corrupt are closed, the real-time layers keep their
promises at any block size, and every fix lands with its regression test.
Every pre-existing document still renders byte-for-byte (the golden corpus is
unchanged); the new validation caps only reject documents that previously
produced NaN, silence, a hang, or ultrasonic output no host can play (a
finite-but-unplayable pitch like `"midi:170"` is an authoring error now —
only unvalidated direct renders see the note-fallback change).

### Deprecated (deleted at 2.0)
- `MixerError::NoSampleRate` — unreachable today: every `Mixer` constructor
  takes the rate up front.

### Fixed
- **Validation rejects the overflow regime.** Pitches resolving to non-finite
  Hz (`"midi:10000"`), huge octave numbers (`"A200000000"` — it could panic the
  parser's i32 arithmetic), `super.detune_cents` above 10 octaves, `fm.ratio`
  above 4096, constant frequencies above 100 kHz (and modulated frequency
  endpoints above ±1 MHz), `rand` rates above 10 000/s (a validated doc could
  hang the renderer for hours), and a non-finite `compress.ratio` all fail
  validation with a clear message instead of rendering NaN or hanging.
- **Silent-authoring-error guards.** A `chain` leading with a processor, a
  bare-processor document root, duplicate automation lanes for one target, and
  an all-zero `env` *modulator* ADSR (the flatten footgun `Node::Env` already
  caught) are validation errors now, not digital silence / dead knobs.
- **Unvalidated documents can't panic the renderer** (the codebase's stated
  contract): bitcrush `bits ≥ 32`, negative `piano_inharm`, sample rates below
  40 Hz, and absurd durations (an allocation abort) are clamped defensively;
  `peak_limit` scrubs non-finite samples instead of passing NaN to the
  encoders; and graph validation is depth-capped and stack-safe for
  programmatically-built documents.
- **Adaptive music is block-size invariant.** Quantized stingers, intensity
  changes, and transitions fired up to one host block early; they now apply at
  their exact frame, and section cross-fades compute from an absolute frame
  count — a 128-frame AudioWorklet and a 512-frame cpal callback render
  identical audio. Also: a mid-fade `transition_to` the fade's target is a
  no-op, requesting the previous section cancels the fade back (click-free),
  a mid-fade switch to a third section lets the in-flight fade complete
  before the onward transition (no hard cut), `AdaptiveMusic` honors
  `AudioSource::reset` through trait objects, and layer cross-fades snap at
  their target instead of asymptoting forever.
- **Runtime engine hardening.** A `PatchId`/`ParamId` from another `Engine`
  resolves inert instead of panicking (the documented contract); a param
  change landing mid-crossfade carries the blend weight instead of restarting
  the fade; a NaN `pan`/`glide` can no longer poison the whole mix; `split(0)`
  floors to a one-frame ring instead of silently never playing.
  `Engine`/`Mixer`/`StreamSource` pre-allocate their scratch (no callback
  allocation for blocks up to 8192 frames), and the docs state the real
  threading contracts (`Engine`'s mutating calls are O(duration) — use
  `split` for real-time).
- **Song compile.** `u32` arithmetic on note steps/bars saturates instead of
  panicking in debug or wrapping in release; a degenerate `bpm` (< 1) clamps
  consistently for duration AND note placement (notes past bar 0 used to drop
  silently); `add_track` slugifies and dedups names like the fluent path.
- **Instrument.** MIDI convention honored: `note_on` with velocity 0 is a
  note-off (it used to leak a stuck silent voice); `transpose` leaves an
  unparseable note as authored instead of substituting a silent A4;
  `with_tremolo(0.0, depth)` is off instead of a constant gain cut.
- **CLI.** MIDI export includes seqs inside `duck` triggers (a doc whose only
  seq was the kick trigger exported note-less) and saturates pathological step
  values instead of overflowing; `tono import` / `tono midi` no longer
  silently overwrite an existing default output file; doc names can't escape
  the output directory; OGG encodes in 8192-sample blocks (a whole-render
  block is orders of magnitude slower in libvorbis).
- **tono-py / tono-desktop.** `stinger` renders off the shared pump lock (it
  used to render under it — an audible dropout); `Engine(sample_rate=0)` is
  rejected; the desktop deck keeps up to two fade generations so rapid doc
  swaps don't hard-cut and pre-allocates its callback scratch; `analyze`
  surfaces PNG-encode failures instead of reporting success with empty images.
- `Patch::instantiate` can't panic on NaN parameter specs (a NaN value skips
  the write); `mutate` clamps to every validation bound (the 30 s delay cap
  and the new detune/rand-rate/frequency caps) so a mutated doc always
  re-validates; `humanize`'s coherent transpose reaches `duck` triggers.

## 1.8.0 — 2026-07-11

The structure release: a full quality review swept every lens, the god-files
split into module directories, the native faces share one cpal shim, and the
long-deprecated names are staged for deletion at 2.0. Every pre-existing
document still renders byte-for-byte (the golden corpus and the
offline/streaming byte-identity fuzz are unchanged).

### Deprecated (deleted at 2.0)
- The 1.6.0 rename aliases, now through two minors: `tono_core::stream`
  (use `player`) and `catalog::Instrument` (use `catalog::Voice`).
- `tono_core::voice` — `EnvGen` lives with its only consumer as
  `instrument::EnvGen`; the module shim keeps the old path valid.
- `tono::audio::write_wav` (mono; the stereo writer is the export path),
  `streaming::is_streamable` (call `StreamGraph::try_from_doc` — it was a
  misdocumented full build-and-discard), and `EffectChain::is_empty`.

### Changed
- `SoundDoc::validate()` is filesystem-free: it no longer stats a sampler's
  `.sf2` path (the same valid doc used to validate differently per machine,
  against the core's no-I/O contract). Loaders call the new pure
  `SoundDoc::sf2_paths()` and check existence themselves — the CLI and the
  Python bindings already do, so their behavior is unchanged. A caller that
  relied on `validate()` to catch a missing file now gets the error at load.
- `Node::Seq`'s per-voice knobs are grouped into `serde(flatten)`ed structs
  (`FmKnobs`/`PluckKnobs`/`PianoKnobs`/`BassKnobs`/`Sf2Knobs`). The JSON wire
  shape is untouched; Rust code that pattern-matched the old flat fields on
  this variant must switch to the structs.
- `tono-desktop` drops its `play` subcommand (use
  `tono_play::play_doc` / `make play`, which streams byte-identically).

### Added
- `tono render` writes the documented `smpl` loop chunk again for
  `playback: loop` WAV exports (silently regressed when the MCP server was
  removed).
- `analysis::spectral_frames` + `stats_with`/`stats_stereo_with`/
  `spectrogram_png_with`: one STFT now feeds both the numeric stats and the
  spectrogram (it was computed twice per render analysis).
- `tono_play::Speaker::open_at` (explicit stream rate) and `Speaker::shared`;
  the desktop deck and the Python engine stream through this one cpal shim.
- `make verify-native` (clippy + tests for the off-CI native crates, examples
  included) with a path-filtered CI workflow; `make play EXAMPLE=<name>`;
  `make python-test` / `python-smoke` / `site` / `version` — CI workflows now
  exec make targets only, and CI validates the golden pins on macOS too.
- The GitHub Pages site gains an architecture & getting-started page.

### Fixed
- Real-time hardening in the streaming path now matches the offline renderer
  exactly (empty `arp` steps, `secs == 0` slides, sub-240 Hz filter clamps).
- The instrument's modulation LFOs derive phase from accumulators instead of
  an absolute `f32` clock — vibrato/tremolo/wobble no longer go steppy after
  ~3 hours of live play (frozen after ~6).
- `AdaptiveMusic`: a stinger with unequal channel lengths could stall the
  spent-stinger cull; the loop play-head no longer overflows on very long
  sessions; `add_stem_set` no longer renders the first stem twice.
- Validation rejects NaN/±inf knobs everywhere (a `1e308` JSON literal used
  to cast silently to `inf` and render garbage), and validates
  `compress.threshold`/`makeup` and `super.freq` like their siblings.
- `describe()` fails loud instead of returning an empty map; review summaries
  are no longer ALL CAPS; the LUFS field's doc says gated (the meter always
  was); `tono-py`'s crate type no longer collides with the root crate's rlib.

## 1.7.0 — 2026-07-11

Audio real-time safety and mixer/adaptive correctness from a full review of the
1.6.0 sprint, plus phase-locked stem sets. Every pre-existing document still
renders byte-for-byte (the golden corpus is unchanged).

### Added
- **Phase-locked stem sets** on `AdaptiveMusic`: `add_stem_set(stems,
  duration_beats)` forces every stem onto one shared loop length (from the tempo,
  or the first stem's natural length without one) so layered intensity
  cross-fades stay sample-aligned and never drift phase; returns the grid length
  in frames. Plus `LoopBuffer::from_doc_len(doc, frames)` — render and loop a doc
  at an exact frame count.
- Off-lock entry points so a real-time wrapper never renders under a lock:
  `AdaptiveMusic::add_section_buffer`, `stinger_stereo`, `stinger_stereo_at`
  (mirroring `add_layer`). The doc-taking `add_section`/`stinger`/`stinger_at`
  now delegate to them.

### Fixed
- **Mixer**: the master fader (`set_bus_gain(MASTER, …)`) was a no-op; sources
  added directly to an FX bus were silently dropped. `write_interleaved` no
  longer reads past a short source slice.
- **Adaptive music**: a transition to the already-current section double-filled a
  buffer (audible speed-up); pending transitions now dedup/supersede; `duck()`
  ramps in instead of stepping (no click) and recovery snaps to unity.
- **Render path**: guarded divide-by-zero / NaN on unvalidated docs (empty `Arp`
  steps, `Slide` `secs == 0`, `soft_limit` `ceil == 0`, low-sample-rate filter
  clamps).
- **Real-time callbacks**: `tono-play` no longer blocks the audio thread on the
  control lock (`try_lock` + silence); all cpal callbacks are wrapped in
  `catch_unwind` (a render panic can no longer unwind across the C frame);
  `tono-py` `Engine::new` no longer leaks the audio thread + stream on a
  pump-spawn failure.

## 1.6.0 — 2026-07-11

The game-audio release: live DSP buses, voice management, beat-quantized
interactive music, and Python bindings — plus a verified bug sweep, a corrected
output stage behind engine revision 4, the native pattern station, and an
organization/API pass. Every pre-existing document still renders byte-for-byte
(a golden corpus now pins this in CI).

### Added
- **Python bindings** (`crates/tono-py`, PyO3): a live `Engine` owning the
  output stream (drum kit, preset instruments, adaptive music, zero-asset patch
  triggers — the audio thread never touches Python), and a numpy pull API
  (`tono.render`, `Patch.render(**params)`), deterministic and CI-testable.
  Build with `make python`; abi3 wheels build in CI.
- **Live DSP effects on mixer buses**: sources feed named buses with insert
  chains (reverb/EQ/compressor/delay/…), post-fader sends into shared FX/return
  buses, and a master chain — all reusing the streaming effect kernels, so a
  bus stays byte-identical to the offline processors. `Mixer::new_at`, `bus`,
  `fx_bus`, `add_to`, `set_bus_effects`, `master_effects`, `set_send`.
- **Voice management**: an opt-in polyphony budget with priority stealing.
  `Engine::set_max_voices`, `Priority` (`LOW`/`NORMAL`/`HIGH`/`CRITICAL`),
  `play_prioritized` / `play_looping_prioritized` / `set_priority`; the victim
  declicks instead of hard-cutting, an outranked voice is denied, and a flood is
  hard-bounded at 2× the budget. `DrumKit::with_max_voices` tunes the kit's cap.
- **Interactive music v2** on `AdaptiveMusic`: a musical clock
  (`set_tempo`/`beats`/`bars`), `Quantize` (`Beat`/`Bar`/`Bars(n)`) scheduling
  for `set_intensity_at` and `stinger_at`, and horizontal **sections**
  (`add_section` + `transition_to`) that cross-fade on the bar — swap "explore"
  for "battle" without a mid-phrase cut.
- **The pattern station** (`make desktop`): a native Tauri studio with
  real-time audio — an FL-style step grid over the catalog instruments,
  click-free live editing, per-track faders, undo, and per-edit
  LUFS/spectrogram feedback. Off the default build and CI.
- `AdaptiveMusic` transport for beat-locked games: `pause`/`resume`/`is_paused`,
  `reset` (rewinds the position clock to 0 and every layer to its loop head),
  and `position_frames()` — the musical clock a game derives its beat position
  from. Plus `duck(depth, release)`, a fast master sidechain for stingers/SFX
  independent of the slower intensity cross-fade.
- `AudioSource::reset()` (default no-op; `LoopBuffer` overrides it) so a
  transport can rewind a looping source to its head.
- `runtime::spsc` generalizes the wait-free split over any `AudioSource`
  (`Pump<S>`; `Controller = Pump<Engine>` unchanged), and
  `runtime::write_interleaved` is the one channel-spread every output adapter
  shares. `tono midi` prints its notes/tracks summary.
- Infrastructure: a `v*` tag now auto-creates its GitHub Release with the
  CHANGELOG section as notes; the showcase site deploys to GitHub Pages.

### Fixed (no rendered bytes change)
- `delay.secs` is bounded — an unbounded value passed `validate()` then aborted
  the process on an arbitrary allocation; constants/modulator endpoints must be
  finite (1e308 rendered NaN buffers); automation lanes are validated.
- The split engine no longer loses frames when over-pumped, and an odd-length
  underrun no longer permanently swaps L/R.
- `StreamSource` carries the bounce's peak-limit gain (streams matched the raw
  graph, playing louder than the bounce); loop/stereo docs fall back to the
  `Player` instead of playing un-looped/un-widened; the `fold` waveshaper can
  no longer hang the audio thread on a non-finite sample.
- Voice stealing declicks with a ~5 ms fade (instrument + drum kit) instead of
  a hard mid-sample cut.
- MIDI export carries velocity, puts drums on channel 10, and no longer drifts
  on non-divisor grids; CLI flags consume their values and unknown options are
  loud errors; morph no longer lerps `engine`/`seed`; a stack of smaller fixes.

### Engine revision 4 (opt-in via the doc's `engine`; new songs stamp it)
- Loudness normalization measures the whole stereo program with ONE shared gain
  (the per-channel stage collapsed asymmetric mixes toward center), using gated
  BS.1770 loudness at the doc's actual sample rate, and enforces `ceiling_dbtp`
  against a real oversampled true-peak estimate.
- Humanize jitter is seeded per note, so chords stop moving as a block.
- `Song` pins `engine`/`version` at creation: saved projects replay
  byte-identically across kernel upgrades.
- All metering (analysis, CLI, desktop) now reads the stereo pair that ships,
  with oversampled true-peak and gated loudness.

### Changed (API)
- `stream` → `player` (deprecated alias kept); `catalog::Instrument` →
  `catalog::Voice` (deprecated alias kept).
- `#[non_exhaustive]` on the enums and builder-structs that grow every release.
- Dead `voice::{BandOsc, Voice, PolySynth}` removed (only `EnvGen` was used).
- `render`/`dsl` split into focused submodules (same public paths);
  `tono_core::prelude` added; the root `tono` crate re-exports every module;
  `missing_docs` is enforced.

### Known limitation (documented)
- Byte-identity holds per platform: platform libm (`sin`/`cos`/`exp`/`powf`)
  differs between macOS-arm64 and linux-x86_64, so the golden pins are
  per-platform. Deterministic transcendental kernels are future work.

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
