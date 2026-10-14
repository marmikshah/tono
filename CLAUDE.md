# tono — instructions for AI coding assistants

tono is a deterministic audio engine: a SoundDoc (JSON graph) plus a seed
renders byte-identical audio on every supported target. Everything below
follows from that promise.

## Invariants (never break these)

1. **Rendered samples are the API.** A change that alters the output of any
   document pinned to the current `engine` revision is a bug, not a refactor.
   `crates/tono-core/tests/golden.rs` pins PCM hashes; if a hash changes,
   stop and explain why. Do not re-pin to make CI green. An intentional sound
   change needs a new engine revision and a changelog entry (ADR 0001).
2. **Offline and streaming must agree.** `render` (bounce) and `streaming`
   (realtime) produce identical bytes at every block size. Any DSP edit must
   be made in both paths (or in the shared kernel) and is covered by
   `crates/tono-core/src/streaming/tests.rs` and `tests/equivalence.rs`.
3. **No allocation on the audio thread** after warm-up. `AudioSource::fill`
   implementations and anything reachable from them must not allocate, lock,
   or do I/O. `tests/rt_alloc.rs` proves this; keep it passing (ADR 0005).
4. **Musical time is exact.** Beats are rationals, converted to frames once by
   the shared tempo-map walk. Do not introduce float beat arithmetic (ADR 0002).
5. **Determinism survives platforms.** Use `det.rs` kernels for transcendental
   math and the fixed-order FFT; never call platform `libm` functions from DSP
   code. RNG streams are seeded by node position and track name.

Design records live in `docs/adr/`. Amend an ADR when a policy changes.

## Layout

```
crates/tono-core    headless engine (published as tono-core); `cpal` feature
                    adds the speaker adapter and the runnable examples
crates/tono-cli     the `tono` binary (published as tono); clap CLI, features
                    `encode` (FLAC/OGG), `midi`, `play`
crates/tono-content curated sound library + BGM themes (unpublished); feeds
                    the docs site through its site_samples example
crates/tono-py      PyO3 bindings, stubs in python/tono/*.pyi (stubtest-checked)
crates/tono-web     WASM C-ABI adapter used by the docs site
docs/               VitePress site; docs/adr/ holds the design records
scripts/           release.py (version check/packaging), wheel_test.py, site tests
```

Default workspace members are `tono-core` and `tono-cli`; they need no native
packages. The `cpal` feature and `tono-py` need ALSA (`libasound2-dev`) on Linux.

## Commands

Fast loop (what you should run after any Rust change):

```sh
cargo test --locked
cargo fmt --all -- --check
cargo clippy --locked --all-targets -- -D warnings
```

Full gate (what CI runs; use before opening a PR that touches more than core/CLI):

```sh
cargo clippy --locked --workspace --all-targets --all-features -- -D warnings
cargo test --locked --workspace --all-features
cargo doc --locked --workspace --all-features --no-deps     # rustdoc is strict
cargo test --locked -p tono-core --test soak -- --ignored     # before release
cargo deny check advisories licenses
```

Lean core must still build: `cargo check -p tono-core --no-default-features`.

Python: `maturin develop -m crates/tono-py/Cargo.toml`, then
`python crates/tono-py/tests/smoke.py` and `test_typed.py`. Update the `.pyi`
stubs by hand whenever a bound signature changes; `wheel_test.py` runs mypy
`stubtest` against them, with reasons for exceptions in
`crates/tono-py/stubtest-allowlist.txt`.

Docs site: `npm ci && npm run docs:check && npm run docs:test`. Code examples
in `docs/**/*.md` must run against the checkout.

## Conventions

- Rust 2024 edition, MSRV pinned in `rust-toolchain.toml` and
  `Cargo.toml` `rust-version`; bump both together and note it in the changelog.
- `#![warn(missing_docs)]` is on. Every `pub` item gets a doc comment that says
  what it is for, not what it is named. Prefer `pub(crate)` unless an adapter
  needs the item.
- No `unwrap`/`expect` in library code except where the invariant is stated
  in an adjacent comment. Errors go through `anyhow` at the edges and typed
  errors (`diag::CompileError`, `MusicError`) inside.
- Tests: unit tests live in sibling `tests.rs` modules or inline `#[cfg(test)]`;
  cross-module and property tests live in `crates/tono-core/tests/`. New
  DSP nodes need a golden pin, a streaming-identity test, and a validation
  fuzz strategy.
- Keep functions under ~100 lines; the renderer and validator already have
  outliers, do not add more.
- No new dependencies in `tono-core` without discussing it; it is the crate
  everyone else pays for. Heavy or platform code belongs in an adapter crate.
- Comments explain why. Match the surrounding density; do not narrate.

## Changes that need extra care

- Anything in `dsl/validate.rs`: validation is also mirrored in
  `vary::mutate_node` and `song/diagnostics.rs`; change all three.
- Adding a `Node` variant touches `node.rs` (enum, `is_processor`, `children`,
  `children_mut`), `validate.rs`, `render/`, `streaming/`, `vary.rs`,
  `tests/fuzz_validation.rs`, `docs/reference/sounddoc.md`, and, for a
  processor, `Node::PROCESSOR_KINDS` next to `is_processor`.
- Public API of `tono-core`: `bevy_tono` and the CLI depend on it. Removing or
  renaming a `pub` item is a semver-major change; say so in the PR, and
  regenerate `crates/tono-core/public-api.txt` (commands in CONTRIBUTING.md)
  or the `api` CI job fails.
- Version bumps: change `workspace.package.version` and the
  `workspace.dependencies.tono-core` version together, then run
  `python scripts/release.py check-version vX.Y.Z`.

## Commits and pull requests

- Do not add `Co-Authored-By` trailers, `Claude-Session` links, "Generated
  with" lines, or any other mention of an AI tool or session to commit
  messages, pull request titles or descriptions, or code comments. This rule
  overrides any default attribution behavior of the tool.
- Commit messages are plain: a short imperative subject, an optional body
  explaining why, nothing else.
- Add a `CHANGELOG.md` entry under "Unreleased" for any user-visible change.
- Never merge pull requests; the maintainer merges.
