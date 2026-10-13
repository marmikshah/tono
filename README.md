# tono

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
cargo install tono
tono generate laser --seed 42 -o laser   # candidates: wav, editable JSON, spectrograms
tono render laser/laser_v0.json           # SoundDoc -> wav + analysis images
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
for a complete Rust and Python walkthrough. Python bindings currently build
from source (see below); they are not on PyPI yet.

## Status

- **Formats:** SoundDoc/Song 2, DSP engine 5, Program 3. Older engine
  revisions keep rendering unchanged. The desktop app resets corrupt projects.
- **Rust:** MSRV is 1.88 (pinned in `rust-toolchain.toml`). Versions follow
  semver; a changed render output for an existing `engine` revision is a bug.
- **Platforms:** CI, release binaries and wheels run on Linux x86_64. macOS and
  Windows are not tested in CI; bug reports and patches for them are welcome.
- **Quality gates:** about 600 tests, including property/fuzz tests, golden
  renders, realtime-allocation checks and soak runs; `clippy -D warnings`,
  strict rustdoc, `cargo deny` and an installed-wheel test on every PR.
- **Authorship:** the code was written with AI assistance under human
  direction and has not had a full line-by-line human review. Review it before
  production use, and report problems through [SECURITY.md](SECURITY.md) or
  issues.

```text
crates/
├── tono-core/     Graphs, DSP, song compiler, streaming, runtime and analysis
│   └── tests/     Regression/property tests and current JSON fixtures
├── tono-web/      Lean WebAssembly adapter using the same core DSP
├── tono-cli/      Audio/MIDI files, feedback images and the tono command
├── tono-play/     Shared cpal speaker adapter and runnable Rust examples
├── tono-desktop/  Tauri pattern station; Rust state and a static webview
└── tono-py/       PyO3 bindings, type stubs, Python tests and examples
docs/             VitePress guides and playable demos
scripts/          Portable release packaging and installed-wheel checks
.github/workflows/ Shared Rust gate, docs, registry/binary releases and wheels
```

Rust uses the pinned 1.88.0 toolchain. Default builds cover the core and CLI:

```sh
cargo run --locked -- --help
cargo run --locked -- generate laser --seed 42
cargo test --locked
cargo fmt --all -- --check
cargo clippy --locked --all-targets -- -D warnings
```

Auditioning needs an output device. Install ALSA, WebKitGTK 4.1,
GTK3/AppIndicator, librsvg and patchelf development packages.

```sh
cargo run --locked -p tono-desktop --release
cargo run --locked -p tono-play --example live_band
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
Rust API is `tono_core::library`; original game starters remain in `generate`.
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
See the [release workflow](docs/project/release-gates.md); scripts provide `--help`.

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md). Security issues: [SECURITY.md](SECURITY.md).
