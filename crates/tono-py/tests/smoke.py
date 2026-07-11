"""End-to-end smoke test for the tono Python bindings.

Covers the deterministic pull API (no audio device needed, so it runs in CI) and
asserts the live-stream classes are exported. The owned-stream Engine is exercised
only when an output device is available.

Run from the repo root after `make python`:
    make python-test
"""

from pathlib import Path

import numpy as np

import tono

EXAMPLES = Path(__file__).resolve().parents[3] / "docs" / "examples"


def test_render_is_deterministic() -> None:
    doc = (EXAMPLES / "blip.json").read_text()
    a = tono.render(doc)
    b = tono.render(doc)
    assert a.dtype == np.float32
    assert len(a) > 0
    assert np.array_equal(a, b), "render must be byte-identical across calls"


def test_patch_params_vary_the_render() -> None:
    patch = tono.Patch((EXAMPLES / "parametric-impact.patch.json").read_text())
    assert set(patch.defaults()) == {"hardness", "size"}
    soft = patch.render(hardness=0.2, size=0.3)
    hard = patch.render(hardness=0.9, size=0.3)
    assert np.array_equal(soft, patch.render(hardness=0.2, size=0.3)), "deterministic"
    assert not np.array_equal(soft, hard), "named params must change the render"


def test_live_classes_exported() -> None:
    for name in ("Engine", "Instrument", "DrumKit", "AdaptiveMusic", "PatchVoice"):
        assert hasattr(tono, name), f"missing {name}"


def test_live_engine_if_device_available() -> None:
    try:
        engine = tono.Engine(48000)
    except Exception as exc:  # no output device (headless CI) — not a failure
        print(f"live engine skipped: {exc}")
        return
    assert engine.sample_rate == 48000
    engine.drumkit().note_on(36, 1.0)
    engine.instrument("warm_lead").note_on("C4", 0.9)
    del engine
    print("live engine OK")


if __name__ == "__main__":
    test_render_is_deterministic()
    test_patch_params_vary_the_render()
    test_live_classes_exported()
    test_live_engine_if_device_available()
    print("all smoke checks passed")
