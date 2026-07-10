"""The pull API: render tono to a numpy array and use it anywhere.

Zero-asset SFX and offline bounces are pure functions of (graph, seed, rate) — so
they are deterministic and testable. Feed the array to sounddevice, a Pygame
buffer, a WAV writer, or an assertion.
"""

from pathlib import Path

import numpy as np

import tono

examples = Path(__file__).resolve().parents[3] / "docs" / "examples"

# A zero-asset SFX patch: infinite variations from named params, no baked audio.
impact = tono.Patch((examples / "parametric-impact.patch.json").read_text())
soft = impact.render(hardness=0.2, size=0.3)
hard = impact.render(hardness=0.9, size=0.3)
print(f"impact  soft peak={np.abs(soft).max():.3f}  hard peak={np.abs(hard).max():.3f}")

# Deterministic — the same bytes every call (on a given platform).
assert np.array_equal(impact.render(hardness=0.6), impact.render(hardness=0.6))

# Or bounce a whole SoundDoc offline.
blip = tono.render((examples / "blip.json").read_text())
print(f"blip: {len(blip)} float32 samples, peak={np.abs(blip).max():.3f}")

# import sounddevice as sd; sd.play(hard, 48000); sd.wait()
