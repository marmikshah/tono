# tono-play — the programmatic playground

A `cpal` speaker so a Rust program can build a sound and hear it in a couple
of lines — the fastest way to audition the engine while developing.

```rust
// play a SoundDoc through the default output device
tono_play::play_doc(&doc)?;
```

Also the shared cpal shim the other native faces (the desktop studio, the
Python extension) build on: device open, the f32 gate, panic containment in
the callback, and channel spreading — one place the platform plumbing lives.

Not part of the default build (heavy platform deps). Run an example
(the recipes: a live band, a song, drums, adaptive music, …):

```sh
cargo run -p tono-play --example live_band
```
