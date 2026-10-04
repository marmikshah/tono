# Run live & embedded

Feed scheduled songs and parameterized effects into your game’s audio system.
First [install the Rust library or Python bindings](/get-started/).

Save each Rust example as `src/main.rs` and run `cargo run`; save each Python
example as `live.py` and run `python live.py`. These examples render in memory
without opening speakers, so they also run in CI.

## Render scheduled audio blocks

Play two bars of bass and lower the gain at the second bar. The output is
three seconds of stereo audio at 48 kHz; the gain change lands at exactly
2 seconds, independent of the size of your render blocks.

::: code-group

```rust [Rust]
use std::sync::Arc;
use tono_core::prelude::*;
use tono_core::runtime::{At, Command, Performance};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut song = Song::new("live-bass", 120.0);
    song.add_voice("bass", &Bass::finger());
    song.add_pattern("riff", 1, vec![
        note(0, 4, "C2"), note(4, 4, "E2"),
        note(8, 4, "G2"), note(12, 4, "C3"),
    ]);
    song.arrange_repeat("bass", "riff", 0, 2);
    let program = song.compile(&CompileOptions {
        sample_rate: Some(48_000), ..Default::default()
    })?;
    let mut performance = Performance::new(Arc::new(program));
    performance.schedule(Command::Play, At::Immediate)?;
    performance.schedule(Command::SetGain(0.4), At::Bar(1))?;
    let mut audio = vec![0.0; 48_000 * 3 * 2]; // Interleaved left/right.
    let frames = performance.fill(&mut audio);
    println!("Rendered {frames} stereo frames");
    Ok(())
}
```

```python [Python]
import tono

song = tono.Song("live-bass", tempo=120)
bass = song.track("bass", tono.instruments.bass())
riff = tono.Pattern(bars=1)
riff.notes(["C2", "E2", "G2", "C3"], durations=1)
song.arrange(bass, riff, bars=range(2))
program = song.compile(sample_rate=48_000)
with tono.Performance(program, headless=True) as performance:
    performance.play()
    performance.set_gain(0.4, at=tono.at_bar(1))
    audio = performance.fill(48_000 * 3)  # Shape: (frames, 2).
    print(f"Rendered {len(audio)} stereo frames")
```

:::

Call `fill` repeatedly for successive blocks. Schedule commands ahead of the
rendered position; `At::NextBar` / `tono.next_bar()` selects the next bar line.
Rust buffers contain **two samples per frame**; Python `fill` takes a frame count.

For speakers, Python `tono.Performance(program)` opens an audio device and
renders in the background; keep it alive while playing and **do not call
`fill` in live mode**. Rust `tono-core` owns no audio device: send its blocks
to your output callback. The repository has a runnable adapter example:
`cargo run -p tono-play --example live_band`
([source](https://github.com/marmikshah/tono/blob/master/crates/tono-play/examples/live_band.rs)).

Check `program.is_streamable()` in Rust or `program.is_streamable` in Python:
unsupported graphs use a pre-rendered buffer. To require native streaming,
compile with `CompileOptions { target: CompileTarget::Runtime, ..Default::default() }`
(`CompileTarget` is in `tono_core::song`) or Python `song.compile(target="runtime")`.

## Vary an effect without shipping WAVs

One patch exposes a pitch parameter. Render it twice: each result contains
9,600 mono samples, with a different pitch.

::: code-group

```rust [Rust]
use std::collections::BTreeMap;
use tono_core::patch::Patch;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let patch: Patch = serde_json::from_str(r#"{
      "doc": {"name":"zap", "duration":0.2, "sample_rate":48000,
              "engine":5, "root":{"type":"sine", "freq":880}},
      "params": [{"name":"pitch", "paths":["root.freq"],
                  "min":100, "max":2000, "default":880}]
    }"#)?;
    let low = patch.render(&BTreeMap::from([("pitch".into(), 220.0)]))?;
    let high = patch.render(&BTreeMap::from([("pitch".into(), 1760.0)]))?;
    assert_ne!(low, high);
    println!("Rendered two effects: {} samples each", low.len());
    Ok(())
}
```

```python [Python]
import tono

patch = tono.Patch('''{
  "doc": {"name":"zap", "duration":0.2, "sample_rate":48000,
          "engine":5, "root":{"type":"sine", "freq":880}},
  "params": [{"name":"pitch", "paths":["root.freq"],
              "min":100, "max":2000, "default":880}]
}''')
low = patch.render(pitch=220)
high = patch.render(pitch=1760)
assert (low != high).any()
print(f"Rendered two effects: {len(low)} samples each")
```

:::

Missing parameters use their defaults; values outside the range are clamped.
Rendering a patch allocates audio: do this before the audio callback, then
queue the samples for playback.

More: [runtime API](https://docs.rs/tono-core/latest/tono_core/runtime/index.html),
[patch API](https://docs.rs/tono-core/latest/tono_core/patch/index.html),
and [a parameterized impact patch](https://github.com/marmikshah/tono/blob/master/crates/tono-core/tests/fixtures/parametric-impact.patch.json).
