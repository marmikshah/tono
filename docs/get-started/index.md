# Install tono

Choose a language, then [make your first sound](/get-started/quickstart). New to audio? [Start with the basics](/get-started/basics) to hear what pitch, tempo, instruments, and patterns do.

You need Rust 1.88 or newer. Python also needs Git and CPython 3.9+.
On Linux, building the Python bindings needs `pkg-config` and ALSA development headers (`libasound2-dev` on Debian/Ubuntu).

Run these commands from the directory where you keep projects:

::: code-group

```sh [Rust]
cargo new tono-demo
cd tono-demo
cargo add tono-core serde_json hound
cargo check
```

```sh [Python]
python3 -m venv .venv
source .venv/bin/activate
python -m pip install tono-py
python -c "import tono; print('tono is ready')"
```

:::

The Python commands use a Unix shell. In Windows PowerShell, activate with `.venv\Scripts\Activate.ps1`. The package is `tono-py` on PyPI and `tono` when imported. Linux x86_64 installs a prebuilt wheel; macOS and Windows build it from the sdist, which needs the Rust toolchain. To work from a checkout instead, run `python -m pip install maturin numpy` and `maturin develop --locked --release -m crates/tono-py/Cargo.toml` inside the clone.

For the optional CLI, run `cargo install tono`, then `tono catalog`.

Contributors working from a checkout can use `cargo add tono-core --path ../tono/crates/tono-core` and `cargo install --locked --path crates/tono-cli` instead.
