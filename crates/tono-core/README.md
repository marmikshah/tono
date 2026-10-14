# tono-core

The headless audio engine behind [tono](https://marmikshah.github.io/tono/):
synthesis graphs, DSP, deterministic rendering, instruments, songs, transport
and analysis. A sound is a small JSON document (a `SoundDoc`) or a Rust value,
and rendering it is a pure function of the document, its seed and the sample
rate, so sounds can be tested, diffed and cached like any other data. The crate
owns no audio device: the `tono` CLI, the native speaker adapters, the Python
bindings and the WebAssembly build all sit on top of it.

```rust
use tono_core::dsl::SoundDoc;
use tono_core::render;

let doc: SoundDoc = serde_json::from_str(r#"{
    "name": "blip", "duration": 0.3, "engine": 5,
    "root": { "type": "mul", "inputs": [
        { "type": "sine", "freq": 880 },
        { "type": "env", "a": 0.002, "d": 0.08, "s": 0.0, "r": 0.05 } ] }
}"#).unwrap();

let a = render::render(&doc);
let b = render::render(&doc);
assert_eq!(a, b); // byte-identical every run
```

## Features

Both features are enabled by default; disable them with
`default-features = false` for the lean build.

- `analysis` — spectral statistics and spectrogram/waveform PNGs (adds
  `image` and `rustfft`).
- `sampler` — SoundFont (`.sf2`) sampler instruments (adds `rustysynth` and
  file I/O).

The [docs.rs](https://docs.rs/tono-core) documentation is built with all
features enabled, so every optional module is listed there.

## Determinism

A document rendered with the same `engine` revision, seed and sample rate
produces the same samples on every supported target, in the offline renderer
and in the real-time streaming path alike. Each document pins an `engine`
revision; older revisions keep rendering unchanged when the engine grows, so a
sound checked into a project never drifts. Golden-render tests in the
repository hold the engine to that promise on every pull request.

## Compatibility

- MSRV: Rust 1.88. Bumps happen in minor releases and are listed in the
  changelog.
- Versioning: semver. The rendered output of a pinned `engine` revision is
  part of the public API and does not change within a major version.

## Links

- [Guides and demos](https://marmikshah.github.io/tono/)
- [Changelog](https://github.com/marmikshah/tono/blob/master/CHANGELOG.md)
- [Repository](https://github.com/marmikshah/tono)
