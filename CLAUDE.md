# tono — repo guide

A headless sound studio driven over MCP: a deterministic synthesis-graph engine
that renders audio and feeds back analysis, so an AI agent can author sound by
tool calls. Same engine also drives a browser playground and an optional native
desktop studio.

## Workspace layout (one core, several faces)

- **`tono-core/`** — the pure engine: the `SoundDoc` graph DSL, DSP,
  deterministic renderer, analysis/critique, graph transforms, the real-time
  `Player`/`voice` (playable synth). No I/O, no MCP, no transport. Compiles
  native **and** `wasm32`.
- **`tono` (root crate, `src/`)** — the MCP server binary + a thin shell
  (file encoders, bank/session persistence, engine emitters, daemon). Depends on
  and re-exports `tono-core`.
- **`tono-wasm/`** — WebAssembly bindings for the browser playground.
  Excluded from `default-members`; built via `make wasm`.
- **`tono-desktop/`** — optional native studio (Tauri window + `cpal`
  real-time audio + MIDI). Excluded from `default-members` and CI; built via
  `make desktop`. Heavy deps (webview/cpal/midir) never touch the default build.

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
- `make wasm` / `make desktop` — the optional faces.
- `make hooks` — install the git hooks (`.githooks/pre-commit`, `pre-push`).

## Conventions

- Clippy clean at `-D warnings`; `cargo fmt` before committing. No dead code.
- Small, focused commits; commit and push as work lands (one concern per commit).
- `tono-core` stays decoupled — no MCP/transport/file-IO leaks into it.
- New tools / capabilities should be expressible across the faces (MCP + UI)
  over the same `SoundDoc`, not bolted onto one.
