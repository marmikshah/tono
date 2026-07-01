# tono (Python)

Deterministic sound synthesis for Python. Author a symbolic synth graph as JSON,
render it to audio samples, or play it live through the speakers — backed by the
same Rust engine as the rest of `tono`, so a doc sounds byte-for-byte identical
whether you render it here, stream it in the desktop studio, or bounce it offline.

## Build

Needs [maturin](https://www.maturin.rs) and a Rust toolchain.

```sh
pip install maturin
maturin develop            # build + install `tono` into the active venv
python examples/blip.py
```

Or, from the workspace root: `make python`.

## Use

```python
import json, tono

doc = json.dumps({
    "name": "blip", "duration": 0.3, "engine": 2,
    "root": {"type": "mul", "inputs": [
        {"type": "sine", "freq": 880},
        {"type": "env", "a": 0.002, "d": 0.08, "s": 0.0, "r": 0.05}]},
})

samples = tono.render(doc)          # list[float], mono, deterministic
left, right = tono.render_stereo(doc)
sr = tono.sample_rate(doc)          # 44100 by default
tono.is_streamable(doc)             # True if it can be played back in real time
tono.play(doc, 0.4)                 # hear it (needs an audio device)
```

`render(doc)` is a pure function of the document — the same JSON always returns
the same samples — so it slots straight into tests and offline pipelines.

## API

| function | returns | notes |
| --- | --- | --- |
| `render(doc, sample_rate=None)` | `list[float]` | mono samples in `[-1, 1]` |
| `render_stereo(doc, sample_rate=None)` | `(list[float], list[float])` | left / right |
| `sample_rate(doc)` | `int` | the doc's render rate |
| `is_streamable(doc)` | `bool` | real-time playback is byte-identical |
| `play(doc, secs)` | `None` | blocking playback on the default device |

The graph vocabulary (nodes, effects, sequencing) is documented with the engine.
