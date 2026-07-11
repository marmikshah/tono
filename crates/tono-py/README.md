# tono (Python)

Live procedural audio, adaptive music, and **zero-asset SFX** for Python games —
a deterministic, Rust-grade synthesis engine. No WAVs to ship.

Two shapes over one engine.

## Owned stream — live playback

```python
import tono

engine = tono.Engine(48000)          # owns a cpal output stream + render thread
kit    = engine.drumkit()
lead   = engine.instrument("warm_lead")

# game loop — control only; the audio thread never touches Python
kit.note_on(36, 1.0)                  # kick
lead.note_on("C4", 0.9)

impact = engine.load_patch(open("impact.patch.json").read())
impact.trigger(hardness=0.8, size=0.3)   # zero baked WAVs

music = engine.adaptive()
music.add_layer(open("combat_stem.json").read(), fade_in_at=0.6)
music.set_intensity(0.9)             # stems swell with the action
```

Runnable version: [`examples/live_pygame.py`](examples/live_pygame.py).

## Pull — render to numpy, integrate anywhere

```python
import tono, numpy as np, sounddevice as sd

# A zero-asset SFX patch: infinite variations from named params, no baked audio.
impact = tono.Patch(open("impact.patch.json").read())
buf = impact.render(hardness=0.7, size=0.3)   # -> np.float32 mono array
sd.play(buf, 48000)

# Deterministic: a pure function of (graph, seed, sample_rate) — testable in CI.
assert np.array_equal(impact.render(hardness=0.7), impact.render(hardness=0.7))

# Or bounce a whole SoundDoc offline.
samples = tono.render(open("blip.json").read())
```

Runnable version: [`examples/render_numpy.py`](examples/render_numpy.py).

## Build

From the repo root: `make python` (maturin develop into the active venv), or
`make wheel` for a release build. CI builds `abi3` wheels (one per platform,
CPython 3.9+) for manylinux / macOS universal2 / Windows. Built with
[PyO3](https://pyo3.rs) + [maturin](https://github.com/PyO3/maturin). Part of
[tono](https://github.com/marmikshah/tono).
