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

The [examples](examples/) are one each for the main flows: `playground`
(build sounds and instruments in code), `live_band` (a drum kit and a piano
in one `Mixer`, notes sent from your code), `lofi` (compose a full song with
the catalog and hear it, or write its SoundDoc), `buses` (live DSP on mixer
buses) and `adaptive` (intensity stems plus a stinger). Python playback
shares `Speaker`: device setup, f32 output, callback panic containment, and
channel adaptation live here. The callback uses a nonblocking source lock and
outputs silence if control holds it; Python feeds it through a separate
pump/sample ring.

This crate is outside the default Cargo build and included in the full Linux
CI gate. Native dependencies and contributor commands are in the
[project README](../../README.md).
