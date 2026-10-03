# ADR 0004: Python bindings

Status: accepted; updated for the current typed and JSON authoring surfaces.

## Context

Python is an authoring and playback surface. Its musical semantics, validation,
and output must match Rust without maintaining a second engine in Python.

## Decision

PyO3 objects hold Rust values and delegate to `tono-core`. Typed Song, Pattern,
Track, Program, and instrument APIs use the same compiler and catalog as Rust.
The SoundDoc/JSON, Patch, Engine, and live playback APIs remain active features
for sound effects and embedding.

Release the GIL around expensive compilation, loading, and rendering operations.
Return owned numpy arrays with explicit mono/stereo shapes. Convert compile and
load failures to Python exceptions; compile diagnostics retain their codes,
messages, and remediation. Ship `.pyi` stubs and `py.typed` in the package.

Live output shares `tono-play::Speaker`. Python control operates on the source
and pump; the native audio callback drains the sample ring without acquiring
the Python GIL or the control mutex.

## Consequences

The installed-wheel smoke and typed suites test real packaging and cross-language
hash/PCM equivalence. CPython 3.9 is the minimum ABI target. Supported automated
builds and manual manylinux wheels are Linux-only; source builds remain available.
The wheels workflow creates artifacts and does not publish to PyPI.

See `crates/tono-py/README.md`, `crates/tono-py/tests/smoke.py`, and
`crates/tono-py/tests/test_typed.py` for the supported development workflow.
