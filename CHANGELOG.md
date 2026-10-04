# Changelog

Released entries describe the behavior and support of that release. Current
workflows and platform support are documented in [README.md](README.md).
Design decisions live in [docs/adr/](docs/adr/README.md).

## Unreleased

### Added
- Original background music collection: 12 themes × three arrangements, seamless four-bar loops, editable Song scores, and browser playback/downloads.
- Sound Studio with draggable instrument layers, a 16-step note grid, envelopes/effects, undo/redo, local drafts, project import/export, and locally rendered WAV downloads.
- Curated asset-free sound library: 64 authored recipes, six editable voicings each, nine categories, and seamless ambience loops through `tono_core::library`.
- Searchable website sound library with variation selection, local previews, editable recipe downloads, and browser-generated stereo WAVs with loop metadata.
- Lean `tono-web` WebAssembly adapter reusing the Rust DSP, bounded worker rendering and PCM caching, plus native/WASM sample-equivalence and full-catalog checks.

### Changed
- Website uses a charcoal audio workspace with lilac accents, a direct sound-browser homepage, consistent controls, and keyboard-accessible global search.
- Effects and music use compact waveform rows, a selected-sound inspector, category filters, variation selectors, saved bookmarks, shareable searches, editable exports, and a persistent preview player. Smaller screens show details inline; waveform cards remain an optional view.
- Sound Studio keeps its continuous sequencer and tabbed device dock visible together on desktop, with grouped instrument browsing, compact transport, an ADSR graph, and project utilities. Smaller screens show a lightweight preset demo instead of the composer, with demo-oriented navigation and library hints.
- Composition examples use a searchable, compact list with direct playback, source links, and downloads.
- Website SFX previews ship recipes and a shared synthesis engine instead of pre-rendered WAV files.
- Builds, CI, CLI release binaries, and Python wheels are Linux-only; Windows/macOS bug patches remain welcome.
- SoundDoc/Song schema 2 and DSP engine 5 are the supported formats; Program format 3 derives document facts and streaming capabilities from their authoritative source.

### Fixed
- Desktop project loading replaces unsupported or corrupt saved data with a fresh current project and reports read/write failures without crashing.
- Program load and scheduled swaps verify the current bundle revision, integrity hash, and resolved document.

### Removed
- Historical DSP/schema implementations, obsolete identity slots, duplicate state, unused tooling, and unused assets.

## 1.10.1 — 2026-08-11

### Fixed
- **Release workflow** — pin `shell: bash` on the asset upload; the Windows runner's pwsh broke it, so the 1.10.0 tag shipped without its Windows binary.

## 1.10.0 — 2026-07-29

**Music-as-code, production-grade.** Compose typed songs in Rust or Python,
compile once into a hashed, validated Program, render offline or run live
with sample-accurate scheduling — byte-identical on every platform. The
composition API is **frozen stable as of rc.1** (schema v2, program v2,
engine v5; see `docs/api-tiers.md` and `docs/release-gates.md`).

### Added
- **Engine revision 5 — cross-platform byte-identity.** Deterministic
  fdlibm-grade transcendental kernels plus fixed-order FFT convolution:
  `engine: 5` documents render bit-for-bit on every supported target; older
  revisions are untouched forever.
- **`Song::compile() -> Program`** — one validation pass collecting every
  diagnostic (stable codes), producing the resolved document, musical
  metadata, bounded resource estimates, and a canonical hash an equivalent
  song reproduces from Rust or Python. Versioned bundles re-verify on load,
  never recompile.
- **Typed Python API** — `tono.Song`/`Pattern`/`Track`/`Program`, the
  `tono.instruments` catalog, numpy renders with the GIL released,
  `py.typed` stubs, structured `CompileError.diagnostics`. Builds from
  source (no prebuilt wheels for now — CI-minutes budget; the pipeline is
  validated, manual-only).
- **`Transport` + `Performance`** — the sample-accurate musical clock and
  the scheduled runtime: seeks by frame/beat/bar/section, loop ranges,
  ramped gain, stingers, crossfaded program swaps, quantized transitions,
  capture/replay, zero allocation on the audio callback. Python:
  `tono.Performance`, live or `headless=True`.
- **The streaming mixer** — schema-v2 `tracks` programs stream
  byte-identical to the offline bounce at any block size: automation lanes
  (linear/step/exp), sidechain duck, buses/sends/inserts, the master chain;
  `StreamGraph::blockers` names any unstreamable part with its fix.
- **Tempo/meter maps, pickup, sections, markers** — exact rational beat
  positions, segment-wise tempo walks, time-signature maps with
  numerator+denominator, anacrusis, named sections/markers carried into the
  Program. Map-less songs compile byte-identically.
- **The `music` module** — `Pitch`/`PitchClass`/`Interval`/`Scale`/`Key`/
  `Chord`/`Voicing` with one strict spelling grammar (no silent guessing);
  inversions, arpeggios, close/drop-2/slash voicings.
- **The pattern algebra** — pure transforms (repeat/concat/layer/slice/
  transpose/stretch/rotate/reverse/quantize, `vel`/`gate`), Euclidean and
  tuplet constructors, deterministic `probability`/`humanize`. Exact or
  loud: off-grid results are errors that name the note.
- **Automation curves + beat-addressed lanes** — `step` and `exp` curves;
  song automation addressed in beats through the tempo map.
- **Mixer buses, inserts, sends, returns** — named buses with insert chains
  and return faders, on tracks roots and songs. Additive: no buses = the
  legacy mix, bit-for-bit.
- **Stems** — `render_stems` decomposes a mix into per-track and per-bus
  contributions that sum exactly to the mix; CLI `--stems`; Python dict with
  `stem_routing`.
- **MIDI through Song** — `tono import --song` / `tono midi --song`.
- **`tono compile SONG.json`** — Program bundles from the shell; `--inspect`
  prints the machine-readable summary; non-zero exit doubles as a CI gate.
- **Exact musical time + typed units + stable ids + structured
  diagnostics** — rational `Beat`s, `Frames`/`Samples`/`SampleRate`/`Hertz`/
  `Decibels`/`Tempo`/`Bars` newtypes, `TrackId`/`PatternId`/`PlacementId`/
  `ParamId`/`BusId`, `Diagnostic`/`CompileError`.
- **Song `seed` + track `mute`/`solo`** — reproducible artifacts, console
  solo semantics. Additive.
- **Prebuilt CLI binaries** on release tags (linux-x86_64, macos-aarch64,
  windows-x86_64, sha256 sidecars).
- **Criterion benches + proptest contract fuzzing** — five render hot-path
  benches (manual CI dispatch); parse/validate/render never-panic
  properties.
- **New nodes: `convolve` and `granular`** — deterministic synthetic-IR
  convolution reverb; seeded grain textures. Both offline-only, and the
  streaming coverage report says so.
- **New nodes: `wavetable` and `tremolo`** — morphing table oscillator
  (four built-in sets); per-sample amplitude modulation. Both stream
  byte-identically.
- **Tracks-level sidechain ducking** — `sidechain: {source, amount, attack,
  release}` per track; validated; streams natively.
- **7 new catalog voices, 5 new presets** — catalog 24 → 31 (Brass, Flute,
  Mallets, Bells), presets 11 → 16, all reachable from the CLI.
- **New seq voices** — `brass`, `flute`, `mallet`, `bell` param-free
  models; stream byte-identically by construction.
- **`tono presets` / `tono catalog`** — list the factory sounds, or render
  a voice's demo with the full render output set.
- **`tono fit` + `tono review`** — deterministic seeded hill-climb toward a
  reference WAV; the grader as a CI-gateable subcommand (non-zero on FAIL),
  with new `footstep`/`powerup` archetypes.
- **Selected-range rendering** — `render_range` by frames or bars, tails
  intact; also in Python.

### Changed
- **House structure** — the CLI moved to `crates/tono-cli` (the root is a
  pure virtual workspace); one contributor contract in `RULE.md`; cargo
  gates in hooks and hosted CI.
- **User docs revamped for scanning** — code-first README, card-flow
  quickstart, complete node-vocabulary cookbook reference, table-driven
  runtime/migration/performance docs, all crate READMEs aligned.

### Deprecated
- **The legacy JSON-string Python API** (`tono.render(doc_json)`,
  `Patch(json)`, `add_layer(doc_json)`) — works through v1.10; the typed
  API is the successor.

### Removed
- **The C ABI and WASM faces** — shipped during the betas, dropped before
  release as overkill (native hosts embed `tono-core`); recorded in
  `docs/release-gates.md`. Can return on demand.

### Fixed
- **A scheduled program swap no longer renders on the audio path** — swap
  sources build at schedule time; the `rt_alloc` gate proves zero
  allocations across a swap.
- **Two RustSec advisories in pyo3** — tono-py builds on pyo3/numpy 0.29;
  `cargo deny` is green.
- **Two contract violations found by the proptest suite** — a
  single-point/NaN automation lane panic; `vary::mutate` overflow on
  uncapped parameters. Both pinned by regression tests.
- **A latent hang in the threaded SPSC soak test.**
- **The deep-review sweep** (engine-5 kernel, streaming mixer, adaptive
  scheduling, MIDI, instrument), each fix pinned by a regression test:
  `det::exp` overflow/underflow edge cases; same-frame adaptive actions now
  fire in schedule order, and a deferred transition lands exactly one frame
  out; MIDI export retimes multi-tempo documents to absolute time; a
  serde-loaded `unison_width > 1` no longer yields negative unison pan
  gains.

### Deferred
- **Opus export** — a production encoder means a libopus system dependency;
  WAV, FLAC, and OGG Vorbis remain the deterministic formats.

## 1.9.0 — 2026-07-19

**The hardening release** — a full-codebase review: input edges that could
render NaN, hang, panic, or silently corrupt are closed, and the real-time
layers keep their promises at any block size. Every pre-existing document
renders byte-for-byte. Stated policy: tono never ships a 2.0 — breaking
changes land in ordinary minors.

### Removed
- The surface deprecated at 1.6.0–1.8.0 (`stream`, `catalog::Instrument`,
  `voice`, `audio::write_wav`, `streaming::is_streamable`,
  `EffectChain::is_empty`, `MixerError::NoSampleRate`).

### Added
- **`tono diff`** — what changed between two docs: loudness, peak,
  brightness, envelope metrics, sample-domain distance.
- **`tono match`** — score a doc against a reference WAV, worst offenders
  first.
- **`tono render --watch`** — re-render on every save.
- **`tono play`** — audition through the speakers (feature-gated `play`).
- **docs/quickstart.md** + the docs index.

### Changed
- **One graph traversal** (`Node::children`/`walk`) — a new variant can
  never be silently skipped; fixed two latent omissions by construction
  (humanize into master chains, transpose into duck triggers).
- **One modulator evaluator** shared by offline and streaming — the two can
  no longer diverge.
- Shared kernel math unified in `dsp.rs`; `adaptive`/`song` split into
  module directories.

### Fixed
- **Validation rejects the overflow regime** — non-finite pitches, absurd
  octave numbers, extreme detune/fm ratios/frequencies, `rand` rates that
  could hang the renderer for hours, non-finite compress ratios.
- **Silent-authoring-error guards** — processor-leading chains,
  bare-processor roots, duplicate automation lanes, all-zero env
  modulators.
- **Unvalidated documents can't panic the renderer** — defensive clamps,
  NaN scrubbing in `peak_limit`, depth-capped stack-safe validation.
- **Adaptive music is block-size invariant** — quantized actions land on
  their exact frame, cross-fades compute from absolute counts, transition
  edge cases are click-free.
- **Runtime hardening** — foreign patch/param handles are inert, mid-fade
  param changes carry the blend weight, NaN pan/glide can't poison the mix,
  pre-allocated callback scratch.
- **Song compile** — saturating step arithmetic, consistent degenerate-bpm
  clamping, track-name slugify/dedup on `add_track`.
- **Instrument** — velocity-0 note-on is a note-off (MIDI convention),
  transpose leaves unparseable notes as authored, zero-rate tremolo is off.
- **CLI** — MIDI export covers duck-trigger seqs and saturates pathological
  steps; no silent output overwrites; doc names can't escape the output
  directory; OGG encodes in blocks.
- **tono-py / tono-desktop** — stinger renders off the pump lock,
  `Engine(sample_rate=0)` rejected, double-buffered desktop fades,
  `analyze` surfaces PNG-encode failures.

## 1.8.0 — 2026-07-11

**The structure release** — a full quality review, god-files split into
module directories, the native faces share one cpal shim, deprecated names
staged for removal. Byte-for-byte stable.

### Deprecated
- The 1.6.0 rename aliases, `tono_core::voice`, `tono::audio::write_wav`,
  `streaming::is_streamable`, `EffectChain::is_empty` — removed in the next
  minor.

### Changed
- `SoundDoc::validate()` is filesystem-free — loaders check `.sf2`
  existence themselves via the new pure `SoundDoc::sf2_paths()`.
- `Node::Seq` per-voice knobs grouped into `serde(flatten)` structs — the
  JSON wire shape is untouched.
- tono-desktop drops its `play` subcommand (use `tono_play::play_doc`).

### Added
- `tono render` writes the `smpl` loop chunk again for looped WAV exports.
- One STFT now feeds both the numeric stats and the spectrogram.
- `tono_play::Speaker::open_at` / `Speaker::shared` — one cpal shim for the
  desktop deck and the Python engine.
- `make verify-native` and friends — CI workflows exec make targets only;
  the Pages site gains an architecture page.

### Fixed
- Real-time hardening in the streaming path now matches the offline
  renderer exactly.
- Modulation LFOs derive phase from accumulators — vibrato/tremolo/wobble
  no longer go steppy after hours of live play.
- `AdaptiveMusic`: stinger cull stall, play-head overflow on long sessions,
  double-rendered first stem.
- Validation rejects NaN/±inf knobs everywhere and covers
  `compress.threshold`/`makeup`/`super.freq`.
- Assorted: loud `describe()`, sentence-case review summaries, the LUFS
  doc, the tono-py crate-type collision.

## 1.7.0 — 2026-07-11

**Real-time safety and mixer/adaptive correctness**, plus phase-locked stem
sets. Byte-for-byte stable.

### Added
- **Phase-locked stem sets** on `AdaptiveMusic` (`add_stem_set`) plus
  `LoopBuffer::from_doc_len` — layered intensity cross-fades stay
  sample-aligned.
- Off-lock entry points (`add_section_buffer`, `stinger_stereo[_at]`) so a
  real-time wrapper never renders under a lock.

### Fixed
- **Mixer** — the master fader works; sources added to an FX bus aren't
  dropped; no past-the-end interleave reads.
- **Adaptive** — no double-fill on a self-transition, transitions
  dedup/supersede, click-free duck ramps.
- **Render path** — divide-by-zero/NaN guards on unvalidated docs.
- **Real-time callbacks** — no lock-blocking in tono-play, `catch_unwind`
  on every cpal callback, no leaked audio thread in tono-py on pump failure.

## 1.6.0 — 2026-07-11

**The game-audio release** — live DSP buses, voice management,
beat-quantized interactive music, Python bindings, the native pattern
station, and engine revision 4 — with a golden corpus now pinning
byte-stability in CI.

### Added
- **Python bindings** (`crates/tono-py`, PyO3) — a live `Engine` owning the
  output stream, plus a numpy pull API (`tono.render`, `Patch.render`):
  deterministic and CI-testable.
- **Live DSP on mixer buses** — insert chains, post-fader sends, a master
  chain, byte-identical to the offline processors.
- **Voice management** — opt-in polyphony budget with priority stealing and
  declicked victims; `DrumKit::with_max_voices`.
- **Interactive music v2** — a musical clock, `Quantize` scheduling, and
  horizontal sections that cross-fade on the bar.
- **The pattern station** (`make desktop`) — a native Tauri studio: step
  grid over the catalog, click-free live editing, undo.
- `AdaptiveMusic` transport (`pause`/`resume`/`reset`/`position_frames`)
  and `duck()`; `AudioSource::reset`; the generalized SPSC pump.
- Tags auto-create GitHub Releases with the changelog section as notes; the
  showcase site deploys to Pages.

### Engine revision 4 (opt-in; new songs stamp it)
- Loudness normalization measures the whole stereo program with one shared
  gain, gated BS.1770 at the doc's rate, true-peak ceiling enforcement.
- Per-note-seeded humanize (chords stop moving as a block); `Song` pins
  engine/version at creation; all metering reads the stereo pair.

### Changed
- `stream` → `player`, `catalog::Instrument` → `catalog::Voice` (aliases
  kept); `#[non_exhaustive]` on the growing enums; dead `voice::*` removed;
  module split + prelude; `missing_docs` enforced.

### Fixed
- Bounded `delay.secs` (an allocation abort), finite constants, validated
  automation lanes.
- Split-engine frame loss and odd-underrun L/R swap; StreamSource
  peak-limit parity; loop/stereo docs fall back to the Player; the `fold`
  shaper can't hang the audio thread.
- Declicked voice stealing; MIDI velocity, channel-10 drums, grid drift;
  CLI flag parsing; morph no longer lerps engine/seed.

### Known limitation
- Byte-identity held per platform only (platform libm differences) — the
  golden pins were per-platform. Retired by engine 5 in 1.10.0.

## 1.5.0 — 2026-07-03

Per-track mixing on the song builder: a `.reverb(0..1)` send, and
`.swing`/`.humanize` per-track overrides of the song-global groove.
Byte-safe — an unset track is identical to before.

## 1.4.0 — 2026-07-03

A composition system on the deterministic core: the **instrument catalog**
(grand piano, e-piano, organ, strings, bass, guitar, drums — with variants)
and the **fluent song builder** (`Song::add(instrument, |t| …)` compiling
to a tracks SoundDoc). Deeper voices, opt-in per variant: the inharmonic
grand-piano model (engine 3), four synthesized drum kits, four bass models,
three guitars. Examples: `lofi`, `band`.

## 1.3.0 — 2026-07-01

The engine becomes a **library + CLI**: the MCP server is removed,
`tono-core` is the published deterministic engine, and `tono render` turns
a SoundDoc into audio plus analysis images. `cargo add tono-core` /
`cargo install tono`.

## 1.2.0 — 2026-06-28

From headless engine to a studio: real-time playback, a browser playground,
an optional native desktop app, track automation, MIDI export, and DSP
engine revisions — byte-stability pinned throughout.

- **`stream::Player`** — the block-filled audition seam, byte-identical to
  an offline bounce; a gated streaming `voice` + `PolySynth` allocator for
  live play.
- **tono-desktop** (Tauri + cpal, optional) — the node patcher with
  real-time audio; play patches from the computer keyboard or a MIDI
  controller.
- **Browser playground** via `tono-wasm` — the same patcher,
  byte-identical in Web Audio.
- **Studio editors** — piano roll for seqs, channel-strip mixer with live
  meters, an inline modal-modes table.
- **`Track.automation`** — gain/pan breakpoint lanes over song time; unset
  tracks render byte-identically.
- **`export_midi`** — every seq to a Standard MIDI File.
- **The `engine` document field** — DSP-kernel revisioning; **engine 1**
  ships ADAA anti-aliased `drive` (per-node opt-out).
- **New nodes** — `modal` resonator bank + `impact` strike exciter (bells,
  coins, wood); `dust` Poisson texture + `rand` random-walk modulator
  (fire, wind, debris).
- **`review_sound`** — deterministic critique against archetype targets and
  the ship checklist, with the sound-review-loop skill;
  `scaffold_layered_sfx` — a band-disciplined 4-layer starting structure.
- **Analyzer** — log-frequency spectrograms; spectral-flatness,
  inharmonicity, and attack-slope metrics.
- **Workspace split** — `tono-core` extracted (no I/O); repo standards pass
  (LICENSE, .editorconfig, hooks, Makefile); the MCP tool surface
  consolidated 30 → 23 with replay unaffected.

## 1.1.0 — 2026-06-12

Compositional authoring: a sound becomes a document of named, addressable
layers on per-layer deterministic RNG streams (schema v2; v1 documents
replay byte-for-byte).

- **Stable layer ids**, start offsets, persisted mute;
  `add_layer`/`set_layer`/`layer_ops`; layer-relative `set_param` paths;
  per-layer contribution stats on every render.
- **Single-pass render** — stereo and analysis from one pass; undo history
  deepens 20 → 100.
- Fixes: failed mutations no longer corrupt history/redo/journal; replay
  preserves version-less steps; rehydrate survives restarts; humanize on
  Tracks roots. Closes 18 issues from an adversarial review.
- Ships the sound-designer skill and three loop-ready game-BGM showcases
  (evening-glade, iron-gauntlet, sunny-steps).

## 1.0.0 — 2026-06-07

First release. A headless sound studio for programmatic clients, driven
over MCP.

- **Instruments** — a polyphonic sequencer with piano/e-piano/organ/
  strings/bass/kit/cowbell/pluck/fm voices, raw band-limited oscillators
  and noise colours, and a SoundFont `sampler`; note-name pitches,
  per-parameter modulators, swing + humanize.
- **Production** — the `tracks` stereo mixing console with a master chain
  and decorrelated reverb; filters/EQ, drive, ringmod,
  chorus/flanger/phaser, compressor, sidechain duck, bitcrush, delay,
  reverb; LUFS-targeted limiting to a true-peak ceiling; seamless loops;
  WAV/FLAC/OGG.
- **The authoring loop** — every render returns analysis plus spectrogram
  and waveform images; JSON-path editing with undo/redo; variations
  (`mutate`/`generate_variants`/`humanize`/`morph`); engine-file manifests
  (Godot/Unity/Bevy).
- **Sessions** — journaled calls; `save_session`/`replay_session`
  reproduce a project byte-for-byte; nine showcases replay in CI.
- **Ops** — stdio + HTTP transports, a self-managing daemon, a one-line
  installer, tagged binaries, dual MIT/Apache-2.0.
