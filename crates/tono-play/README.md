# tono-play — native audio output

A `cpal` speaker for Rust programs: build a sound and hear it through the
default output device.

```rust
// Play a SoundDoc for 0.6 seconds.
tono_play::play_doc(&doc, 0.6)?;
```

Run a live example from the workspace root:

```sh
cargo run --locked -p tono-play --example live_band
```

The [examples](examples/) cover instruments, voice management, mixing,
songs, and adaptive music. Desktop and Python playback share `Speaker`:
device setup, f32 output, callback panic containment, and channel adaptation
live here. The callback uses a nonblocking source lock and outputs silence if
control holds it. Python and the `voices` example use a separate pump/sample
ring; the desktop plays a pre-rendered deck through the same speaker adapter.

This crate is outside the default Cargo build and included in the full Linux
CI gate. Native dependencies and contributor commands are in the
[project README](../../README.md).
