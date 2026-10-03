# Run live & embedded

Depend on the pure [`tono-core`](https://github.com/marmikshah/tono/tree/master/crates/tono-core) crate and your game gets
the same deterministic engine the CLI and studio render with: a **live
runtime** (sample-accurate song playback, a voice engine, a mixer) and
**patches** — `SoundDoc` templates with named parameters that render
per-instance variations at runtime (an impact that scales with collision
force, a footstep that varies by surface). Zero baked WAV files: a sound is a
function of its inputs, never a recorded asset.

## Run a song live in 60 seconds

A compiled `Program` plays through a **`Performance`**: a sample-accurate
transport plus a bounded, submission-ordered command queue, so musical
scheduling never depends on a game loop or an OS timer waking on the
boundary:

```rust
use tono_core::runtime::{At, Command, Performance};

let mut perf = Performance::new(program.into());      // program: a compiled tono_core Program
perf.schedule(Command::Play, At::Immediate)?;
perf.schedule(Command::SetGain(0.8), At::NextBar)?;   // ramped, click-free
perf.transition_to_section("chorus", At::NextBar)?;
// perf.fill(&mut block) on the render side — each command lands on its exact frame.
```

Commands carry musical positions (beats, bars, markers, section names),
resolved to exact frames at schedule time. Identical timestamps execute in
submission order; a full queue (4096 commands) is a defined rejection, never
a stall.

Everything live implements one trait — `runtime::AudioSource` ("fill this
interleaved-stereo buffer") — so your output adapter never depends on a
concrete engine type.

## Pick the runtime object

| Object | What it's for |
|---|---|
| `runtime::Performance` | Play compiled songs live: transport, scheduling, program swaps, stingers, replay. |
| `runtime::Engine` | Load docs/patches as resources, spawn instances, tween parameters, cap polyphony with priority stealing. |
| `runtime::Mixer` | Route any set of sources through buses with live insert chains (reverb/EQ/compressor) and post-fader sends. |
| `adaptive::AdaptiveMusic` | Intensity-layered stems, beat-quantized transitions, stingers on the downbeat, sidechain ducking. |
| `instrument::Instrument` | A polyphonic, playable voice over any patch (`note_on`/`note_off`, sustain, bends, per-note params) for live keyboards. |
| `Engine::split(ring_frames)` → `(Controller, Renderer)` | The wait-free seam for a real audio thread: a `Controller` for your game loop, a lock-free `Renderer` for the callback — no mutex ever touches the audio thread. |

A runnable, compile-checked `Engine`/`Mixer` example — a drum kit and a
piano, notes sent live from code:
`cargo run -p tono-play --example live_band`
([source](https://github.com/marmikshah/tono/blob/master/crates/tono-play/examples/live_band.rs)).

API detail lives on [docs.rs](https://docs.rs/tono-core).

## Drive the Performance

- **Transport**: play/pause/stop, seeks by frame/beat/bar, loop ranges by
  frames or bars; position reads back as frames, beats, or bars — the same
  exact walks the compiler used, so they can't disagree.
- **Playback**: native streaming for schema-v2 `tracks` programs
  (byte-identical to the offline bounce at any block size — automation,
  sidechains, buses and all); everything else plays the pre-rendered
  bounce. Seeks rebuild deterministically.
- **Swaps**: `swap_to` crossfades programs at a frame, beat, bar, or
  section boundary — a rejected target keeps the last valid program
  running.
- **Stingers**: `stinger(doc, gain, at)` renders at schedule time, never on
  the render path.
- **Metrics**: `metrics()` reads frames rendered, commands
  executed/dropped, max queue depth, swaps, stingers — safe to read off the
  audio callback, no formatting or allocation on it.
- **Replay**: `start_capture`/`stop_capture` records timestamped commands;
  replaying them reproduces the take bit-for-bit.

Schedule at least one pump quantum (plus the ring depth, for split/threaded
use) ahead of the musical point — the queue executes at exact frames, so
anything earlier is free and anything later is late.

Python gets the same surface as `tono.Performance` — live (speaker) or
`headless=True` for tests and servers (`.fill(frames)` renders manually),
with `tono.next_bar()`-style scheduling helpers.

## Patch a sound, not a WAV

A `Patch` is a template document plus parameters, each bound to one or more
graph paths. Instantiating with runtime values bakes a concrete `SoundDoc`,
which the renderer turns into audio:

```
Patch (shipped JSON)  +  { hardness: 0.8, size: 1.3 }  →  SoundDoc  →  samples
```

Determinism holds: the same patch and the same values always render
byte-identically, so a recorded performance reproduces exactly, and you can
bake to WAV offline and stream the identical thing in-engine.

### Render one from Rust

```toml
# Cargo.toml
[dependencies]
tono-core = "1"
```

```rust
use std::collections::BTreeMap;
use tono_core::patch::Patch;

// Load a patch shipped with your game (authored in the studio or another tool).
let patch: Patch = serde_json::from_str(include_str!("../assets/impact.patch.json"))?;

// On each collision, render a unique hit from the contact parameters.
fn on_collision(patch: &Patch, force: f32, object_size: f32) -> Vec<f32> {
    let values = BTreeMap::from([
        ("hardness".into(), force),       // harder strike = brighter
        ("size".into(),     object_size), // bigger object = longer ring
    ]);
    patch.render(&values).unwrap()        // mono samples, ready for your audio backend
}
```

- `patch.render(&values)` is the one call: missing parameters fall back to
  their `default`, out-of-range values clamp, a bad path is a clear error —
  never a corrupt graph.
- Want the concrete `SoundDoc` instead of mono samples (to stereoize, loop,
  or analyse it)? Use `patch.instantiate(&values)`.

### Write the patch format

```json
{
  "doc": { "...": "a normal SoundDoc template" },
  "params": [
    { "name": "hardness", "paths": ["root.stages[0].hardness"],
      "min": 0.1, "max": 1.0, "default": 0.6 },
    { "name": "size",
      "paths": ["root.stages[1].modes[0].decay", "root.stages[1].modes[1].decay"],
      "min": 0.1, "max": 1.5, "default": 0.5 }
  ]
}
```

- One parameter can drive several paths at once (here `size` rings every
  modal partial longer).
- Paths are the same ones `tono_core::edit::describe` / `apply_ops` use, so
  a tool can design the sound in the studio, read off the paths, and emit
  the patch.
- Worked example:
  [`crates/tono-core/tests/fixtures/parametric-impact.patch.json`](https://github.com/marmikshah/tono/blob/master/crates/tono-core/tests/fixtures/parametric-impact.patch.json).

## Ship it anywhere

`tono-core` handles synthesis, compilation and transport without owning an
audio device. Its optional `sampler` feature reads `.sf2` files by path. It
compiles to native and game targets — so one patch plays identically in the studio and the shipped game.
