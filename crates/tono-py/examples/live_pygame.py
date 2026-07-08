"""Live procedural audio for a Python game — the whole thing in ~10 lines.

No WAVs, no compiler: `pip install tono`, then drive the engine from your loop.
This example uses plain `time.sleep` in place of a real game loop; drop the same
calls into a Pygame / Arcade / Ren'Py loop instead.
"""

import time

import tono

engine = tono.Engine(48000)          # owns a cpal output stream + render thread
kit = engine.drumkit()               # a General MIDI drum kit
lead = engine.instrument("warm_lead")  # a factory-preset instrument

for beat in range(8):
    kit.note_on(36, 1.0)             # kick on every beat — GIL-safe control
    if beat % 2:
        lead.note_on("C4", 0.9)      # a note on the off-beats
    time.sleep(0.25)
