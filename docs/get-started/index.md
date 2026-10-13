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
git clone https://github.com/marmikshah/tono.git
cd tono
python3 -m venv .venv
source .venv/bin/activate
python -m pip install maturin numpy
maturin develop --locked --release -m crates/tono-py/Cargo.toml
python -c "import tono; print('tono is ready')"
```

:::

The Python commands use a Unix shell. In Windows PowerShell, activate with `.venv\Scripts\Activate.ps1`.

For the optional CLI, run `cargo install tono`, then `tono catalog`.

Contributors working from a checkout can use `cargo add tono-core --path ../tono/crates/tono-core` and `cargo install --locked --path crates/tono-cli` instead.
