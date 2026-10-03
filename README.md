# tono

tono is a deterministic Rust audio engine for sound effects, instruments,
songs and adaptive game music, with native playback and Python integrations.

[Docs and demos](https://marmikshah.github.io/tono/) · [Architecture](docs/public/architecture.html) · [Changelog](CHANGELOG.md) · [Rust API](https://docs.rs/tono-core) · [MIT](LICENSE)

Formats: SoundDoc/Song 2, DSP engine 5, Program 3; the desktop resets corrupt projects.

Builds, CI and release artifacts are Linux-only. I no longer have the budget
for Windows or macOS builds; patches for bugs on those platforms are welcome.

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
