# tono — repo guide

A deterministic synthesis-graph engine that renders audio and feeds back
analysis (a spectrogram + waveform + numeric stats), so a sound can be authored
by inspection — from Rust, the `tono render` CLI, or a live keyboard. The same
engine powers a real-time streaming renderer, a playable-instrument layer, a
song/arrangement layer, adaptive game music, a native desktop studio, and a
programmatic playground.

## Product voice & versioning

- tono stands on its own: a **developer-friendly audio engine with live
  playback at runtime**. Docs, changelogs, and PRs describe it in its own
  vocabulary (SoundDoc, Patch, Engine, layers/sections) — never reference
  other products by name or by analogy.
- **Docs are split by audience.** User-facing text (README, docs/, crate
  READMEs, example headers) answers with cargo/maturin/pip commands and
  runnable examples — the `Makefile` is the *contributor* interface and its
  targets appear only here (CLAUDE.md) and in the architecture guide.
- **There is never a 2.0.** Breaking changes land in ordinary 1.x minors, and
  deprecated surface is removed directly in the next minor — no long-lived
  deprecation shims. The byte-identity promise below is a product guarantee,
  independent of version numbers.
- The Bevy face lives in the separate `bevy_tono` repo — update it there,
  don't grow a new adapter crate here.

## Workspace layout (one core, several faces)

The root is the `tono` crate (the CLI); the sub-crates live under `crates/`.

- **`crates/tono-core/`** — the pure engine: the `SoundDoc` graph DSL, DSP,
  deterministic renderer, analysis/critique, graph transforms, the byte-identical
  **streaming** real-time renderer, the **runtime** (`Engine`/`Mixer`/`AudioSource`),
  the **instrument** / **drum-kit** / **adaptive-music** layers, and the **song**
  arrangement layer. No I/O, no transport; pure compute.
- **`tono` (root crate, `src/`)** — a thin CLI shell: the `tono render` command,
  audio-file encoders, the analysis image writer, and MIDI export. Depends on and
  re-exports `tono-core`.
- **`crates/tono-desktop/`** — the native desktop studio (Tauri window + `cpal`
  real-time audio + MIDI keyboard input). Excluded from `default-members` and CI;
  built via `make desktop`. Heavy deps (webview/cpal/midir) never touch the default build.
- **`crates/tono-play/`** — the programmatic playground: a `cpal` speaker so a Rust
  program can build a sound/instrument and hear it in a couple of lines. Excluded
  from `default-members`/CI; run via `make play EXAMPLE=<name>`.
- **`crates/tono-py/`** — the PyO3 Python bindings (render + live `Engine` stream).
  Excluded from `default-members`/CI; built via `make python` / `make wheel`,
  smoke-tested by `make python-test`. Build-from-source only — never published to
  PyPI (the name is taken).

## The invariant that matters

Rendering is a pure function of `(graph, seed, sample_rate)` → **byte-identical**
audio. A golden corpus (`crates/tono-core/tests/golden.rs`) pins the exact
rendered hashes of representative documents — and the docs/examples recipes —
in CI, so a kernel change that shifts the offline and streaming paths together
still fails loudly. Do not change synthesis math in a way that breaks existing
renders — gate byte-changing kernel upgrades behind the document `engine`
revision. The real-time audition path must stay byte-identical to an offline
bounce.

Known limitation: byte-identity currently holds **per platform**. The DSP calls
platform libm (`sin`/`cos`/`exp`/`powf`), whose last bits differ between
macOS-arm64 and linux-x86_64, so the golden pins are per-platform (integer-RNG /
PolyBLEP / rational-filter content is identical everywhere; transcendental
content is not). Making the invariant truly cross-platform means deterministic
transcendental kernels behind a future engine revision.

## Build / test

- `make verify` — exactly what CI runs: `fmt --check` + clippy (`-D warnings`) +
  tests. The pre-push hook runs this. `make check` is the mutating version.
- `make pre-commit-checks` — the lint gate (fmt + clippy) alone.
- `make verify-native` — the gate for the off-CI crates: touching tono-desktop /
  tono-play / tono-py? This is your gate — plain `make verify` does not compile
  them (they are non-default workspace members). CI runs it via the Native
  workflow when those crates change.
- `make desktop` / `make play` — the native faces (heavy deps, off the default build).
- `make hooks` — install the git hooks (`.githooks/pre-commit`, `pre-push`).

## Release checklist

Every release, in order (the `release` target enforces clean master + tags
from `Cargo.toml`; CI publishes to crates.io and builds wheels on the tag):

1. Bump **both** version fields in the root `Cargo.toml` together:
   `workspace.package.version` and `workspace.dependencies.tono-core`
   (cargo strips `path` at publish time and pins the crates.io dep to the
   version field — a mismatch ships a CLI built against last release's core).
2. Retitle the CHANGELOG's `## Unreleased` to `## X.Y.Z — <date>` (the
   Release workflow extracts the notes by that exact header).
3. Confirm `cargo publish --dry-run -p tono-core` passes. The `-p tono`
   dry-run only resolves once `tono-core` X.Y.Z is on crates.io, so it runs
   after step 4, not before.
4. `make release` (tags `vX.Y.Z`, pushes; CI publishes `tono-core` then
   `tono`, creates the GitHub Release, and builds the tag-only wheels).

## Conventions

- Clippy clean at `-D warnings`; `cargo fmt` before committing. No dead code.
- Small, focused commits; commit and push as work lands (one concern per commit).
- `tono-core` stays decoupled — no transport/file-IO leaks into it.
- New capabilities should be expressible across the faces (CLI + code + UI)
  over the same `SoundDoc`, not bolted onto one.
