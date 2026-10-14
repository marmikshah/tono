# tono Python bindings

Compose typed songs, render SoundDocs and parameterized patches to numpy,
or control live instruments, drum kits and adaptive music through native audio.
CPython 3.9+ is supported with abi3 wheels.

Build from a repository checkout in an activated virtual environment:

```sh
python -m pip install maturin numpy
maturin develop -m crates/tono-py/Cargo.toml
python crates/tono-py/tests/smoke.py
python crates/tono-py/tests/test_typed.py
maturin build --locked --release -m crates/tono-py/Cargo.toml
```

Linux builds need ALSA development headers. The manual Wheels workflow
produces install-tested Linux x86_64 artifacts.

## Type stubs

`python/tono/__init__.pyi` and `instruments.pyi` are hand-maintained; when a
binding's name, parameters or defaults change, update the stub alongside it.
`scripts/wheel_test.py` (the CI wheel gate) runs mypy's `stubtest`, which
compares the installed extension against the stubs; the few things PyO3 cannot
expose are listed with reasons in `stubtest-allowlist.txt`. To run it locally
against a built wheel directory, or directly in an environment where
`maturin develop` has installed the package:

```sh
python scripts/wheel_test.py dist
python -m pip install mypy && python -m mypy.stubtest tono --allowlist crates/tono-py/stubtest-allowlist.txt
```

See the [Python guide](https://marmikshah.github.io/tono/guides/python),
[live runtime guide](https://marmikshah.github.io/tono/guides/live),
[runnable API tours](https://github.com/marmikshah/tono/tree/master/crates/tono-py/examples), and
[complete reference scores](https://github.com/marmikshah/tono/tree/master/crates/tono-content/examples/python).
The root [README](https://github.com/marmikshah/tono) covers contributor gates
and releases. tono is [MIT licensed](https://github.com/marmikshah/tono/blob/master/LICENSE).
