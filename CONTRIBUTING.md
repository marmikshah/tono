# Contributing to tono

Bug reports, fixes and docs improvements are welcome, including patches for
macOS and Windows, which CI does not cover. For larger changes, open an issue
first so the design can be agreed before you invest time.

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
- Keep commits focused, and describe what changed and why in the PR.
- Disclose substantial AI assistance in the PR description; you remain
  responsible for understanding and testing the change.

## Releases

Maintainers only. See [docs/project/release-gates.md](docs/project/release-gates.md).

By contributing you agree that your work is licensed under the [MIT License](LICENSE).
