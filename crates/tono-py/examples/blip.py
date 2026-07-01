"""Make a sound and play it, from Python.

    maturin develop         # build + install `tono` into the current venv
    python examples/blip.py

`tono` speaks the same `SoundDoc` JSON as the engine everywhere else: a graph of
signal nodes (`sine`, `square`, `noise`, `fm`, `seq`, …) wired through combinators
(`mul`, `mix`, `chain`) and effects. Rendering is deterministic — the same doc
always yields the same samples — so `render()` is a pure function you can test.
"""

import json

import tono


def blip(freq: float) -> str:
    """A short plucked tone: a sine shaped by a fast-decaying amp envelope."""
    return json.dumps(
        {
            "name": "blip",
            "duration": 0.35,
            "engine": 2,
            "root": {
                "type": "mul",
                "inputs": [
                    {"type": "sine", "freq": freq},
                    {"type": "env", "a": 0.002, "d": 0.12, "s": 0.0, "r": 0.05, "punch": 0.3},
                ],
            },
        }
    )


if __name__ == "__main__":
    doc = blip(880.0)

    samples = tono.render(doc)  # list[float], mono, in [-1, 1]
    print(f"rendered {len(samples)} samples at {tono.sample_rate(doc)} Hz")
    print(f"peak {max(abs(x) for x in samples):.3f}, streamable: {tono.is_streamable(doc)}")

    # A little melody, played out loud (needs an audio device).
    for note in (523.25, 659.25, 783.99, 1046.50):
        tono.play(blip(note), 0.30)
