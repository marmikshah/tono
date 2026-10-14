# Contributing to tono

Bug reports, fixes and docs improvements are welcome, including patches for
macOS and Windows: CI runs the core and CLI tests there, but the speaker
adapter and the Python extension are only checked on Linux. For larger
changes, open an issue first so the design can be agreed before you invest time.

## Setup

Rust uses the pinned toolchain in `rust-toolchain.toml`. The default workspace
members (core and CLI) need no native packages:

```sh
cargo test --locked
cargo fmt --all -- --check
cargo clippy --locked --all-targets -- -D warnings
```

The full gate, including the core `cpal` speaker feature and the Python
extension, is in the [README](README.md) and runs in CI on every pull request.
Linux needs the ALSA development package (`libasound2-dev`).

Criterion benchmarks live in `crates/tono-core/benches/render.rs`
(`cargo bench --locked -p tono-core --bench render`, or append `-- --test`
for a quick execution check). Compare timings on the same hardware and
feature set; CI proves allocation behaviour, not wall-clock budgets.

## Ground rules

- **Determinism is the product.** Do not change rendered samples for an
  existing `engine` revision. Behavior changes need a new engine revision; see
  [ADR 0001](docs/adr/0001-determinism.md) and the golden tests.
- **Realtime code does not allocate.** See ADR 0005 and
  `crates/tono-core/tests/rt_alloc.rs`.
- Add tests with the change: regression or property tests in
  `crates/tono-core/tests/`, Python checks in `crates/tono-py/tests/`.
- Update `CHANGELOG.md` under "Unreleased" and any affected docs. Docs code
  examples must run against the checkout.
- **Public API changes are explicit.** `crates/tono-core/public-api.txt` is a
  snapshot of the `tono-core` API and CI fails when the crate no longer
  matches it. If you add, remove or change a public item, regenerate the
  snapshot in the same PR so the diff is part of the review, and add a
  changelog line for the change (under "Breaking" when it removes or renames
  something). It needs the nightly date and `cargo-public-api` version pinned
  in `.github/workflows/ci.yml`:

  ```sh
  rustup toolchain install nightly-2026-06-26 --profile minimal
  cargo install --locked cargo-public-api@0.52.0
  RUSTUP_TOOLCHAIN=nightly-2026-06-26 cargo public-api -p tono-core --all-features -ss --color never > crates/tono-core/public-api.txt
  ```
- Keep commits focused, and describe what changed and why in the PR.
- Disclose substantial AI assistance in the PR description; you remain
  responsible for understanding and testing the change.

## Releases

Maintainers only. Every version below 2.0 may break the API; the changelog
is where that is disclosed, so the release is only as good as its notes.

1. Turn the "Unreleased" changelog entries into a `## X.Y.Z — date` section.
   Every public API removal or rename, CLI change and document-format change
   goes under "Breaking" with a migration note. Older entries are history and
   stay as they are.
2. Set `workspace.package.version` and `workspace.dependencies.tono-core`
   in `Cargo.toml` to the same value (member crates inherit it), then run
   `python scripts/release.py check-version vX.Y.Z`.
3. Confirm `crates/tono-core/public-api.txt` is the API being released and
   that the `semver` CI job's report matches the "Breaking" section. Run the
   full gate from the README plus `cargo test --locked -p tono-core --test
   soak -- --ignored` and `cargo deny check advisories licenses`.
4. Merge, then tag the reviewed commit on `master` as `vX.Y.Z` and push the
   tag. `publish.yml` publishes `tono-core` to crates.io, waits for the index,
   then publishes `tono`; versions already on crates.io are skipped on a
   retry. `release.yml` creates the GitHub Release from that version's
   changelog section (missing notes fail the workflow) and attaches the CLI
   binaries for Linux x86_64, macOS (Apple silicon and Intel) and Windows
   x86_64 with SHA-256 sidecars. Python wheels are built by the manual
   `wheels.yml` workflow and are not published to PyPI.
5. A published crate version cannot be unpublished, only yanked. Speaker
   playback and real SoundFont output need a host with a device; record any
   check that could not be run in the release review.

By contributing you agree that your work is licensed under the [MIT License](LICENSE).
