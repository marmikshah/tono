# tono — repo guide

A headless sound studio driven over MCP: a deterministic synthesis-graph engine
that renders audio and feeds back analysis, so an AI agent can author sound by
tool calls. The same engine also powers a real-time streaming renderer, a
playable-instrument layer, a native desktop studio, and a programmatic playground.

## Workspace layout (one core, several faces)

The root is the `tono` crate (the MCP server); the sub-crates live under `crates/`.

- **`crates/tono-core/`** — the pure engine: the `SoundDoc` graph DSL, DSP,
  deterministic renderer, analysis/critique, graph transforms, the byte-identical
  **streaming** real-time renderer, the **runtime** (`Engine`/`Mixer`/`AudioSource`),
  the **instrument** layer (polyphonic, pitched, playable), and the **song**
  arrangement layer. No I/O, no MCP, no transport; pure compute.
- **`tono` (root crate, `src/`)** — the MCP server binary + a thin shell
  (file encoders, bank/session persistence, engine emitters, image I/O, daemon).
  Depends on and re-exports `tono-core`.
- **`crates/tono-desktop/`** — the native desktop studio (Tauri window + `cpal`
  real-time audio + MIDI keyboard input). Excluded from `default-members` and CI;
  built via `make desktop`. Heavy deps (webview/cpal/midir) never touch the default build.
- **`crates/tono-play/`** — the programmatic playground: a `cpal` speaker so a Rust
  program can build a sound/instrument and hear it in a couple of lines. Excluded
  from `default-members`/CI; run via `make play`.

## The invariant that matters

Rendering is a pure function of `(graph, seed, sample_rate)` → **byte-identical**
audio. Session files replay byte-for-byte; example recipes are replay-tested in
CI. Do not change synthesis math in a way that breaks existing renders — gate
byte-changing kernel upgrades behind the document `engine` revision. The
real-time audition path must stay byte-identical to an offline bounce.

## Build / test

- `make verify` — exactly what CI runs: `fmt --check` + clippy (`-D warnings`) +
  tests. The pre-push hook runs this. `make check` is the mutating version.
- `make pre-commit-checks` — the lint gate (fmt + clippy) alone.
- `make desktop` / `make play` — the native faces (heavy deps, off the default build).
- `make hooks` — install the git hooks (`.githooks/pre-commit`, `pre-push`).

## Conventions

- Clippy clean at `-D warnings`; `cargo fmt` before committing. No dead code.
- Small, focused commits; commit and push as work lands (one concern per commit).
- `tono-core` stays decoupled — no MCP/transport/file-IO leaks into it.
- New tools / capabilities should be expressible across the faces (MCP + UI)
  over the same `SoundDoc`, not bolted onto one.
