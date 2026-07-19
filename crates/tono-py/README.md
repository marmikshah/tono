# tono-py — the Python bindings

The `tono` Python extension: the deterministic engine plus a live runtime,
from Python. Two surfaces:

- **Offline render** — `tono.render(doc_json)` → numpy arrays (deterministic,
  CI-testable), `Patch.render(**params)` for parametric patches.
- **Live engine** — `tono.Engine(sample_rate=48000)` owns a cpal output stream
  and a render thread: instruments (`engine.instrument("warm_lead")`), a GM
  drum kit (`engine.drumkit()`), SFX patches (`engine.load_patch(json).trigger(...)`),
  and an adaptive-music bed (`engine.adaptive()`).

## Build from source

Never published to PyPI (the name is taken) — build it here:

```sh
pip install maturin
make python        # maturin develop → the `tono` module in your env
make python-test   # the determinism smoke test
make wheel         # a release abi3 wheel → target/wheels/
```

abi3-py39: one wheel per platform covers every CPython 3.9+.

Requires stable Rust (see `rust-version` in the workspace `Cargo.toml`) and a
CPython 3.9+ install.
