# tono

[![CI](https://img.shields.io/github/actions/workflow/status/marmikshah/tono/ci.yml?branch=master&label=CI)](https://github.com/marmikshah/tono/actions/workflows/ci.yml)
[![crates.io](https://img.shields.io/crates/v/tono-core.svg)](https://crates.io/crates/tono-core)
[![docs.rs](https://img.shields.io/docsrs/tono-core)](https://docs.rs/tono-core)
[![MSRV](https://img.shields.io/crates/msrv/tono-core)](#versioning-and-stability)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)

tono is a deterministic audio engine written in Rust. It synthesizes sound
effects, instruments, songs and adaptive game music from small JSON recipes
(SoundDocs) and Rust or Python code. The same recipe renders byte-identical
audio on every supported target, so sounds can be tested in CI.

Use it to generate game sound effects without asset files, compose and compile
songs as code, run adaptive music live, or render in the browser through
WebAssembly.

[Docs and demos](https://marmikshah.github.io/tono/) · [Architecture](docs/public/architecture.html) · [Changelog](CHANGELOG.md) · [Rust API](https://docs.rs/tono-core) · [MIT](LICENSE)

## Quickstart

```sh
cargo install tono                        # WAV only; see the features note below
tono generate laser --seed 42 -o laser   # candidates: wav, editable JSON, spectrograms
tono render laser/laser_v0.json           # SoundDoc -> wav + analysis images
```

The default build writes WAV and needs no C toolchain. Cargo features add the
rest: `midi` (`tono midi` and `tono import`), `encode` (`--format flac|ogg`),
and `play` (`tono play` through the speakers):

```sh
cargo install tono --features midi,encode
```

A SoundDoc is plain JSON. Render one from Rust with `tono-core`:

```rust
use tono_core::{dsl::SoundDoc, render::render};

let doc: SoundDoc = serde_json::from_str(r#"{
  "name": "blip", "duration": 0.25, "sample_rate": 48000, "engine": 5, "seed": 7,
  "root": { "type": "mul", "inputs": [
    { "type": "sine", "freq": 880 },
    { "type": "env", "a": 0.002, "d": 0.08, "s": 0.0, "r": 0.05 }
  ] }
}"#)?;
doc.validate()?;
let samples: Vec<f32> = render(&doc); // identical on every platform
```

See [your first sound](https://marmikshah.github.io/tono/get-started/quickstart)
for a complete Rust and Python walkthrough. The Python bindings are on PyPI as
`tono-py` (`pip install tono-py`, imported as `tono`); Linux x86_64 gets a
wheel, other platforms build from the sdist with a Rust toolchain.

## Status

- **Formats:** SoundDoc/Song 2, DSP engine 5, Program 3. Older engine
  revisions keep rendering unchanged.
- **Rust:** MSRV is 1.88 (pinned in `rust-toolchain.toml`). Versions follow
  semver; a changed render output for an existing `engine` revision is a bug.
- **Platforms:** CI runs the core and CLI tests on Linux, macOS and Windows,
  and releases attach CLI binaries for all three. The speaker adapter, the
  Python extension and the wheels are built on Linux x86_64 only.
- **Quality gates:** about 600 tests, including property/fuzz tests, golden
  renders, realtime-allocation checks and soak runs; `clippy -D warnings`,
  strict rustdoc, `cargo deny` and an installed-wheel test on every PR.
- **Authorship:** the code was written with AI assistance under human
  direction and has not had a full line-by-line human review. Review it before
  production use, and report problems through [SECURITY.md](SECURITY.md) or
  issues.

## Versioning and stability

- Every version below 2.0 is AI generated and breaking: any minor release
  (1.11, 1.12, …) may change or remove public API, CLI subcommands and
  document fields. Each such change is listed under "Breaking" in the
  [changelog](CHANGELOG.md) with a migration note, and the `semver` CI job
  reports it. Pin an exact version (`tono-core = "=1.11.0"`) if you need a
  fixed API.
- The rendered output of any document pinned to an `engine` revision is part
  of the API in every release: it never changes for that revision. A
  different render for the same document, seed and sample rate is a bug.
- MSRV bumps happen in minor releases and are noted in the changelog.

```text
crates/
├── tono-core/     Graphs, DSP, song compiler, streaming, runtime and analysis
│   ├── examples/  Runnable Rust examples (speaker ones need `--features cpal`)
│   └── tests/     Regression/property tests and current JSON fixtures
├── tono-content/  Authored sound library, BGM themes, site exporter and Python scores
├── tono-web/      Lean WebAssembly adapter using the same core DSP
├── tono-cli/      Audio/MIDI files, feedback images and the tono command
└── tono-py/       PyO3 bindings, type stubs, Python tests and examples
docs/             VitePress guides and playable demos
scripts/          Portable release packaging and installed-wheel checks
.github/workflows/ Shared Rust gate, docs, registry/binary releases and wheels
```

Rust uses the pinned 1.88.0 toolchain. Default builds cover the core and CLI
without an audio device:

```sh
cargo run --locked -- --help
cargo run --locked -- generate laser --seed 42
cargo test --locked
cargo fmt --all -- --check
cargo clippy --locked --all-targets -- -D warnings
```

Auditioning needs an output device: the core `cpal` feature (the shared
speaker adapter behind `tono play`, the Python bindings and the examples) is
off by default. On Linux, install the ALSA development package
(`libasound2-dev`).

```sh
cargo run --locked -p tono-core --features cpal --example live_band
cargo test --locked --workspace --all-features
cargo clippy --locked --workspace --all-targets --all-features -- -D warnings
```

Python supports CPython 3.9+. In an activated virtual environment:

```sh
python -m pip install maturin numpy
maturin develop -m crates/tono-py/Cargo.toml
python crates/tono-py/tests/smoke.py
python crates/tono-py/tests/test_typed.py
```

Docs need Node 22.12+ and Rust. Use `python3` where `python` is unavailable.

```sh
npm ci
rustup target add wasm32-unknown-unknown
npm run docs:dev
npm run docs:check
npm run docs:build
npm run docs:test
```

The [sound library](https://marmikshah.github.io/tono/sounds) contains 64
procedural designs with six voicings each. The site serves editable recipes
and renders previews and WAV downloads locally through WebAssembly. The
Rust API is `tono_content::library` in the unpublished `tono-content` crate;
original game starters remain in `tono_core::generate`.
The [background music library](https://marmikshah.github.io/tono/bgm) adds 12
original themes in three arrangements each, with editable Song scores.
Use the [Sound Studio](https://marmikshah.github.io/tono/create) to add instrument
layers, write notes, shape envelopes and effects, and export your own sounds.

Ready PRs check pinned/latest Rust, Linux native builds and installed wheels.
Before release run `cargo deny check advisories licenses` and
`cargo test --locked -p tono-core --test soak -- --ignored`.
Bump both versions in Cargo.toml; with Python 3.11+ run
`python scripts/release.py check-version vX.Y.Z`, then tag the reviewed commit.
Tags publish core then CLI and GitHub binaries. Wheels are a manual workflow;
See the release section of [CONTRIBUTING.md](CONTRIBUTING.md); scripts provide `--help`.

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md). Security issues: [SECURITY.md](SECURITY.md).
