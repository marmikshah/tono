# tono (Python)

Live procedural audio, adaptive music, and **zero-asset SFX** for Python games —
a deterministic, Rust-grade synthesis engine. No WAVs to ship, no compiler on the
user's machine, `pip install tono`.

Two shapes over one engine.

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
samples = tono.render(open("blip.sound.json").read())
```

## Owned stream — live playback (coming in this crate)

```python
import tono

engine = tono.Engine(48000)          # owns a cpal output stream + render thread
kit    = engine.drumkit()
lead   = engine.instrument("lead")

# game loop — control only; the audio thread never touches Python
kit.note_on(36, 1.0)                  # kick
lead.note_on("C4", 0.9)

music = engine.adaptive("combat_stems.json")
music.set_intensity(0.9)             # stems swell with the action
```

Built with [PyO3](https://pyo3.rs) + [maturin](https://github.com/PyO3/maturin),
shipped as an `abi3` wheel (one per platform, CPython 3.9+). Part of
[tono](https://github.com/marmikshah/tono).
