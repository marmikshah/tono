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

See the [Python guide](https://marmikshah.github.io/tono/guides/python),
[live runtime guide](https://marmikshah.github.io/tono/guides/live), and
[runnable examples](https://github.com/marmikshah/tono/tree/master/crates/tono-py/examples).
The root [README](https://github.com/marmikshah/tono) covers contributor gates
and releases. tono is [MIT licensed](https://github.com/marmikshah/tono/blob/master/LICENSE).
