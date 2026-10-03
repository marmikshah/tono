"""tono — deterministic synthesis, typed songs and live audio from Python.

Song, Pattern, Track and Program share the Rust composition model and
canonical program hash. SoundDocs and parameterized Patch templates render
sound effects to numpy. Engine, Instrument, DrumKit, AdaptiveMusic and
PatchVoice provide native playback; Performance schedules compiled songs.
"""

from ._tono import *  # noqa: F401,F403 — re-exports the whole native surface
from . import _tono as _native
import sys as _sys

# `instruments` is a native submodule of the extension; register it under the
# package name so `import tono.instruments` works as well as the attribute.
_sys.modules[__name__ + ".instruments"] = _native.instruments
