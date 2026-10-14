# Changelog

Released entries describe the behavior and support of that release. Current
workflows and platform support are documented in [README.md](README.md).
Design decisions live in [docs/adr/](docs/adr/README.md).

## Unreleased

## 1.11.0 — 2026-10-14

The workspace trim: the desktop app, the unpublished play crate and the
authored content leave the published crates, the CLI moves to clap with
optional features, and CI gains cross-platform, MSRV, public-API and semver
checks. The 1.x line may break in minor releases; every break is listed here.

### Breaking
- `tono_core::library` and `tono_core::bgm` moved to the unpublished
  `tono-content` crate in this repository. Depend on it by git path if you
  used the recipes or themes; `tono-core` no longer ships authored content.
- The `tono` CLI parses arguments with clap: `tono` with no subcommand exits
  with status 2 and the help text, FLAC/OGG output needs the `encode` feature
  and MIDI import/export the `midi` feature (`cargo install tono --features
  encode,midi,play`; WAV is always available). The undocumented `diff`,
  `match`, `fit` and `review` subcommands are gone.
- The cpal speaker adapter moved from the unpublished `tono-play` crate into
  `tono-core` behind the `cpal` feature (off by default); the CLI's `play`
  feature and the Python bindings use it. The runnable examples now live in
  `crates/tono-core/examples` and need `--features cpal`.
- Removed unused public API: `edit::describe`/`morph` and their map types,
  `vary::humanize`, `music::Voicing`, the `ids` module, the `units` newtypes
  (`Bars`, `Decibels`, `Hertz`, `Samples`), and the `Serialize` derives on
  `Recipe` and `Composition`.
- `StreamBlocker::ModulatedFilter` (diagnostic T1507) is gone: modulated filters and gains stream. Code matching on the enum must drop that arm.
- `Engine::play_prioritized` and `play_looping_prioritized` are replaced by `Engine::spawn(patch, Spawn { looping, priority })`; `play` and `play_looping` are unchanged.
- Nine node types are removed: `wavetable`, `bitcrush`, `downsample`, `ringmod`, `flanger`, `phaser`, `duck`, `convolve` and `granular`. A document using one fails to parse with the unknown type named; use a mixer `sidechain` instead of `duck`, and `chorus`/`tremolo` for modulation. Documents that never used them render byte-identically (schema 2 and engine 5 stay; see ADR 0001). With them go `dsl::WavetableKind`, `StreamBlocker::OfflineEffect` (diagnostic T1508) and the FFT in `det`.
- Program format 3: `Program` no longer duplicates `schema_version`,
  `engine_version` and `warnings`, and `ProgramMeta`/`TrackMeta` drop the
  fields now derived from the document. Bundles compiled by 1.10 must be
  recompiled.
- SoundDoc/Song schema 2 and DSP engine 5 are the only supported revisions;
  older documents are rejected with a diagnostic instead of being upgraded.

### Added
- Beginner audio lessons covering pitch, tempo, patterns, instruments, and sound graphs, with a playable phrase to hear how each setting affects the result.
- Original background music collection: 12 themes × three arrangements, seamless four-bar loops, editable Song scores, and browser playback/downloads.
- Sound Studio with draggable instrument layers, a 16-step note grid, envelopes/effects, undo/redo, local drafts, project import/export, and locally rendered WAV downloads.
- Curated asset-free sound library: 64 authored recipes, six editable voicings each, nine categories, and seamless ambience loops through `tono_content::library`.
- Searchable website sound library with variation selection, local previews, editable recipe downloads, and browser-generated stereo WAVs with loop metadata.
- Lean `tono-web` WebAssembly adapter reusing the Rust DSP, bounded worker rendering and PCM caching, plus native/WASM sample-equivalence and full-catalog checks.
- CI runs the test suite on Linux, macOS and Windows, checks the pinned MSRV, compares the `tono-core` public API against `crates/tono-core/public-api.txt`, reports semver breakage against the latest crates.io release, and runs mypy `stubtest` against the Python type stubs.
- Crate metadata for docs.rs and crates.io (homepage, documentation, README for `tono-core`, trimmed package contents) and a versioning policy in the README.
- The Python bindings are published to PyPI as `tono-py` on every release tag (Linux x86_64 wheel plus an sdist; the import name stays `tono`).

### Changed
- Website guides use concise, runnable Rust and Python examples with a language preference shared across code blocks and pages.
- Website uses a charcoal audio workspace with lilac accents, a direct sound-browser homepage, consistent controls, and keyboard-accessible global search.
- Effects and music use compact waveform rows, a selected-sound inspector, category filters, variation selectors, saved bookmarks, shareable searches, editable exports, and a persistent preview player. Smaller screens show details inline; waveform cards remain an optional view.
- Sound Studio keeps its continuous sequencer and tabbed device dock visible together on desktop, with grouped instrument browsing, compact transport, an ADSR graph, and project utilities. Smaller screens show a lightweight preset demo instead of the composer, with demo-oriented navigation and library hints.
- Composition examples use a searchable, compact list with direct playback, source links, and downloads.
- Website SFX previews ship recipes and a shared synthesis engine instead of pre-rendered WAV files.
- GitHub Releases attach CLI binaries for Linux x86_64, macOS (Apple silicon and Intel) and Windows x86_64, built with default features. Python wheels are still built for Linux x86_64 only.
- The SoundDoc reference shows the external tagging of modulators and the object form of the `stereo` field.
- `Node::PROCESSOR_KINDS` lists the processor node tags; the Python bindings validate bus effects against it instead of a private copy, and their typed-song module is split into `convert`, `instruments`, `pattern`, `song` and `program`.
- `dsl::bounds` holds every numeric validation bound and the shared tempo-map rule (`tempo_map_problems`; `song::meter_map_problems` for meter maps): the validator, `vary::mutate`, the song compiler's diagnostics, the renderer's defensive clamps and the Python `set_tempo_map`/`set_meter_map` read one definition. The validator is split into header, mixer, layer, sidechain, bus and routing checks with unchanged messages.
- One runtime clock and scheduler: `AdaptiveMusic` quantizes on the same `Transport` a `Performance` runs on (`Transport::constant`, `next_beat`, `next_bar`), both queue their frame-stamped actions on one timeline type, and stingers are one pre-rendered one-shot type. The performance's master gain and swap crossfade ride the same per-frame ramp as instance faders.
- Live bus and instrument master chains built with `EffectChain::stereo` give a reverb the decorrelated 0/23 stereo tail the document mixer gives its buses, so a bus sounds the same live and bounced (the runtime `Mixer` and `Instrument` master reverbs are now stereo).
- The streaming identity suite is a corpus: one document per node family, modulator, seq voice and mixer shape, swept across block sizes by one test each; the seq voice character checks are one table.
- The song compiler builds each track's `seq` node and the document directly instead of through a JSON round trip; a test checks every catalog voice knob still lands on the node.
- One DSP evaluator: the offline bounce now runs the same block-based kernels as the streaming renderer (one block as long as the document instead of per-sample recursion), so the two can no longer drift; every golden hash is unchanged. The mixer captures layer stats and stems in the same pass. Filters, EQ and `gain` with a modulated cutoff or amount stream natively (the coefficients follow the modulator per sample), so they no longer block streaming.

### Fixed
- Program load and scheduled swaps verify the current bundle revision, integrity hash, and resolved document.

### Removed
- The `tono-desktop` Tauri app; the web Sound Studio replaces it. Linux CI no longer needs WebKitGTK, AppIndicator, librsvg or patchelf.
- Historical DSP/schema implementations, obsolete identity slots, duplicate state, unused tooling, and unused assets.
- The `docs/project` pages (roadmap, release workflow, performance notes); release steps and benchmark instructions now live in `CONTRIBUTING.md`.

## 1.10.1 — 2026-08-11

### Fixed
- **Release workflow** — pin `shell: bash` on the asset upload; the Windows runner's pwsh broke it, so the 1.10.0 tag shipped without its Windows binary.

## 1.10.0 — 2026-07-29

**Music-as-code, production-grade.** Compose typed songs in Rust or Python,
compile once into a hashed, validated Program, render offline or run live
with sample-accurate scheduling — byte-identical on every platform. The
composition API is **frozen stable as of rc.1** (schema v2, program v2,
engine v5).

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
  release as overkill (native hosts embed `tono-core`). Can return on demand.

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
renders byte-for-byte. Stated policy: breaking changes land in ordinary
minors.

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

## Older releases

Versions before 1.8.0 are listed on [GitHub releases](https://github.com/marmikshah/tono/releases).
