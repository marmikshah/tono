# In-engine runtime — parametric SFX

The same deterministic engine that renders in the MCP server and the desktop
studio also runs **inside your game**. A game depends
on the pure [`tono-core`](../crates/tono-core) crate, ships a **patch** (a
`SoundDoc` template + named parameters), and renders per-instance variations at
runtime — an impact that scales with collision force, a footstep that varies by
surface — with **zero baked WAV files**. No DAW can do this; it's the payoff of
the deterministic, headless design.

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
tono-core = { git = "https://github.com/marmikshah/tono" }
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
partial longer). Paths are the same ones `set_param` / `describe_sound` use, so
an agent can design the sound in the studio, read off the paths, and emit the
patch. A worked example: [`docs/examples/parametric-impact.patch.json`](examples/parametric-impact.patch.json).

## Where it runs

`tono-core` is pure (no I/O, no MCP, no transport) and compiles to native and
game targets — so one patch plays identically in the studio and the shipped game.
