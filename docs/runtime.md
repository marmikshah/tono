# Embedding tono — the live runtime and parametric patches

The same deterministic engine that renders on the command line and in the
desktop studio also runs **inside your game**. A game depends on the pure
[`tono-core`](../crates/tono-core) crate and gets two things: a **live
runtime** (an embeddable engine/mixer that serves real-time audio) and
**patches** — `SoundDoc` templates with named parameters that render
per-instance variations at runtime, an impact that scales with collision
force, a footstep that varies by surface, with **zero baked WAV files** — a
sound is a function of its inputs, never a recorded asset.

## The live runtime in 60 seconds

Everything live implements one trait — `runtime::AudioSource` ("fill this
interleaved-stereo buffer") — so your output adapter never depends on a
concrete engine type:

```rust
use tono_core::runtime::{Engine, Priority};

let mut engine = Engine::new(48_000);
engine.set_max_voices(32);                                  // polyphony budget
let music = engine.load(&bgm_doc);
engine.play_looping_prioritized(music, Priority::CRITICAL); // never stolen
```

- **`Engine`** — load docs/patches as resources, spawn instances, tween
  parameters, cap polyphony with priority stealing.
- **`Mixer`** — route any set of sources through buses with live insert
  chains (reverb/EQ/compressor) and post-fader sends.
- **`Engine::split(ring_frames)`** — the wait-free seam for a real audio
  thread: a `Controller` for your game loop, a lock-free `Renderer` for the
  callback. No mutex ever touches the audio thread.
- **`adaptive::AdaptiveMusic`** — intensity-layered stems, beat-quantized
  section transitions, stingers on the downbeat, sidechain ducking.
- **`instrument::Instrument`** — a polyphonic, playable voice over any patch
  (note_on/note_off, bends, mod wheel) for live keyboards.

API detail lives on [docs.rs](https://docs.rs/tono-core); the
[architecture guide](https://marmikshah.github.io/tono/architecture.html)
explains how the pieces compose. The rest of this page covers patches.

## The idea

A `Patch` is a template document plus parameters, each bound to one or more graph
paths. Instantiating with runtime values bakes a concrete `SoundDoc`, which the
renderer turns into audio:

```
Patch (shipped JSON)  +  { hardness: 0.8, size: 1.3 }  →  SoundDoc  →  samples
```

Determinism holds: the same patch and the same values always render
byte-identically, so a recorded performance reproduces exactly, and you can bake
to WAV offline and stream the identical thing in-engine.

## Using it from a game (Rust)

```toml
# Cargo.toml
[dependencies]
tono-core = "1"
```

```rust
use std::collections::BTreeMap;
use tono_core::patch::Patch;

// Load a patch shipped with your game (authored in the studio / by an agent).
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

`patch.render(values)` is the one call: missing parameters fall back to their
`default`, out-of-range values clamp, and a bad path is a clear error — never a
corrupt graph. Use `patch.instantiate(values)` if you want the concrete
`SoundDoc` (e.g. to stereoize, loop, or analyse it) instead of mono samples.

## The patch format

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

One parameter can drive several paths at once (here `size` rings every modal
partial longer). Paths are the same ones `tono_core::edit::describe` / `apply_ops` use, so
an agent can design the sound in the studio, read off the paths, and emit the
patch. A worked example: [`docs/examples/parametric-impact.patch.json`](examples/parametric-impact.patch.json).

## Where it runs

`tono-core` is pure (no I/O, no transport — apart from the opt-in `sampler`
feature, which reads `.sf2` files by path) and compiles to native and
game targets — so one patch plays identically in the studio and the shipped game.
