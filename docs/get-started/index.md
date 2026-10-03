# Install tono

Three faces, one engine — pick the one you need; every one renders the same bytes.

## The CLI

```sh
cargo install tono
```

Prove it works — list the 31 built-in voices:

```sh
tono catalog
```

## The Rust library

```sh
cargo add tono-core
```

Prove it works — compile a song and render the mix:

```rust
use tono_core::catalog::Bass;
use tono_core::prelude::*;

let mut song = Song::new("demo", 120.0);
song.add_voice("bass", &Bass::finger());
song.add_pattern("riff", 1, vec![note(0, 2, "C2"), note(4, 2, "G2")]);
song.arrange_repeat("bass", "riff", 0, 1);
let program = song.compile(&CompileOptions::default()).expect("compiles");
let (left, right) = program.render_stereo();
```

## The Python bindings

Build from a repository checkout with Maturin in a virtual environment.
The manual Wheels workflow also produces install-tested Linux x86_64 artifacts:

```sh
python -m pip install maturin numpy
maturin develop -m crates/tono-py/Cargo.toml
```

Prove it works — the determinism smoke test:

```sh
python3 crates/tono-py/tests/smoke.py
```

Next: the [ten-minute quickstart](/get-started/quickstart).
