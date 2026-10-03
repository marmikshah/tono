"""Tests for the typed tono API (Song/Pattern/Track/Program + instruments).

The headline check is the cross-language contract: the reference song built
here compiles to the same canonical Program hash as the identical Rust song in
crates/tono-core/tests/equivalence.rs — Rust owns semantics, so equivalent
songs must hash equal from either language. The rest covers render shapes and
determinism, bundle round-trips, the alpha.2 composition surface (tempo/meter
maps, pickup, sections/markers, buses, automation, pattern ops, harmony), and
the structured error paths.

Run from the repo root after `maturin develop -m crates/tono-py/Cargo.toml`:
    python3 crates/tono-py/tests/test_typed.py
"""

import json
import tempfile
import time
from fractions import Fraction
from pathlib import Path

import numpy as np

import tono

# The cross-language pin — crates/tono-core/tests/equivalence.rs asserts the
# same value. If this changes, change both or the languages disagree.
REFERENCE_HASH = 0x11EE57F2CF7C7447


def reference_program() -> "tono.Program":
    """The reference song, mirrored line-for-line by the Rust test."""
    song = tono.Song("night-drive", tempo=122.0)
    bass = song.track("bass", tono.instruments.bass("finger"))
    drums = song.track("drums", tono.instruments.drums("tr808"))
    riff = tono.Pattern(bars=1)
    riff.notes(["C2", "C2", "Eb2", "G2"], durations=0.5)
    beat = tono.Pattern(bars=1)
    beat.hit("kick", beats=[0, 2])
    beat.hit("snare", beats=[1, 3])
    song.arrange(bass, riff, bars=range(4))
    song.arrange(drums, beat, bars=range(4))
    return song.compile(sample_rate=48000)


def test_equivalence_hash_matches_rust() -> None:
    program = reference_program()
    # Cross-language contract — crates/tono-core/tests/equivalence.rs pins the
    # same hash for the same song built through the Rust API.
    assert program.hash == REFERENCE_HASH, (
        f"hash {program.hash:#x} != pinned {REFERENCE_HASH:#x} — "
        "the Rust and Python song builders diverge"
    )
    assert program.sample_rate == 48000
    assert [t["name"] for t in program.tracks] == ["bass", "drums"]
    assert [t["wave"] for t in program.tracks] == ["bass", "kit"]
    assert [t["notes"] for t in program.tracks] == [16, 16]
    assert program.estimates["events"] == 32
    assert program.is_streamable is True  # v2 tracks roots stream byte-identically (alpha.2)
    assert program.warnings == [], program.warnings


def test_render_shapes_and_determinism() -> None:
    program = reference_program()
    stereo = program.render()
    assert stereo.dtype == np.float32
    assert stereo.ndim == 2 and stereo.shape[1] == 2, "shape (frames, 2)"
    assert stereo.flags["C_CONTIGUOUS"]
    frames = stereo.shape[0]
    expected = program.duration_seconds * program.sample_rate
    assert abs(frames - expected) <= 1.0, f"{frames} frames vs duration {expected}"
    assert np.array_equal(stereo, program.render()), "render must be byte-identical"

    mono = program.render_mono()
    assert mono.dtype == np.float32
    assert mono.shape == (frames,)
    # The mono render is the mid of the stereo pair.
    assert np.array_equal(mono, 0.5 * (stereo[:, 0] + stereo[:, 1]))


def test_bundle_round_trips() -> None:
    program = reference_program()
    assert tono.Program.from_json(program.to_json()).hash == program.hash
    with tempfile.TemporaryDirectory() as tmp:
        path = Path(tmp) / "night-drive.program.json"
        program.save(str(path))
        assert tono.Program.load(str(path)).hash == program.hash


def test_error_paths() -> None:
    assert issubclass(tono.CompileError, tono.TonoError)

    # Arranging onto an unknown track fails the compile with structured
    # diagnostics (one per problem, each carrying code and path).
    song = tono.Song("bad", tempo=100.0)
    song.track("bass", tono.instruments.bass())
    pattern = tono.Pattern()
    pattern.note("C2")
    song.arrange("nope", pattern)
    try:
        song.compile()
        raise AssertionError("expected tono.CompileError")
    except tono.CompileError as exc:
        diags = exc.diagnostics
        assert isinstance(diags, list) and len(diags) == 1
        assert diags[0]["code"] == "T1001"
        assert diags[0]["path"] == "arrangement[0].track"
        assert diags[0]["severity"] == "error"
        assert "nope" in diags[0]["message"]
        assert "remediation" in diags[0]

    # Unknown instrument variant / drum name are plain ValueErrors.
    for bad_call in (
        lambda: tono.instruments.bass("nope"),
        lambda: tono.instruments.piano("honky_tonk"),  # the slug is honky-tonk
        lambda: tono.Pattern().hit("conga", beats=[0]),
    ):
        try:
            bad_call()
            raise AssertionError("expected ValueError")
        except ValueError as exc:
            assert "expected one of:" in str(exc), str(exc)

    # A durations list must match the pitch count.
    try:
        tono.Pattern().notes(["C4", "E4"], durations=[0.5])
        raise AssertionError("expected ValueError")
    except ValueError as exc:
        assert "durations" in str(exc)

    # A corrupt/invalid bundle is a TonoError, not a crash or a ValueError.
    for garbage in ("not json {", "{}"):
        try:
            tono.Program.from_json(garbage)
            raise AssertionError("expected tono.TonoError")
        except tono.TonoError:
            pass


def test_voice_builders_chain_and_pattern_repr() -> None:
    voice = tono.instruments.bass("pick")
    assert voice.gain(0.8) is voice, "builder methods return self"
    assert voice.pan(-0.25).reverb(0.4).swing(0.5).humanize(0.1) is voice
    assert voice.name == "pick bass"
    assert voice.wave == "bass"
    assert repr(voice) == "Voice('pick bass')"

    pattern = tono.Pattern(bars=2)
    pattern.note("C4", at=0.5, duration=0.25, gain=0.9)
    pattern.chord(["E4", "G4"])
    assert pattern.bars == 2
    assert repr(pattern) == "Pattern(bars=2, notes=3)"


def test_packaging_and_legacy_surface() -> None:
    assert (Path(tono.__file__).parent / "py.typed").exists(), "PEP 561 marker ships"
    assert callable(tono.render), "the legacy JSON render still works"
    import importlib

    importlib.import_module("tono.instruments")  # importable as a submodule too


def test_stems_render() -> None:
    song = tono.Song("stem-test", tempo=120)
    bass = song.track("bass", tono.instruments.bass("finger"))
    keys = song.track("keys", tono.instruments.piano("grand"))
    riff = tono.Pattern(bars=1)
    riff.notes(["C2", "G2"], durations=1.0)
    song.arrange(bass, riff, bars=1)
    song.arrange(keys, riff, bars=1)
    program = song.compile(sample_rate=48_000)
    stems = program.render_stems()
    assert set(stems) == {"bass", "keys"}, stems.keys()
    for arr in stems.values():
        assert arr.shape[1] == 2 and arr.dtype == np.float32
        assert np.abs(arr).max() > 0, "each stem sounds"
    # The stem sum is the mix the master chain hears (all master-routed here).
    total = sum(stems.values())
    mix = program.render()
    assert total.shape == mix.shape
    assert program.stem_routing == {}, "no bus routing in this song"
    # A selected range is a slice of the full render; bars resolve through the meter.
    piece = program.render_range(frames=(1000, 5000))
    assert np.array_equal(piece, mix[1000:5000])
    bar1 = program.render_range(bars=(0, 1))
    assert bar1.shape[0] == 48_000 * 60 * 4 // 120, "one bar at 120 BPM is 2 s"
    try:
        program.render_range(frames=(0, 1), bars=(0, 1))
        raise AssertionError("both range forms must be rejected")
    except ValueError:
        pass


# --- alpha.2: temporal maps, structure, buses, automation, ops, harmony ---


def _doc_track(program: "tono.Program", index: int = 0) -> dict:
    """The compiled document's mixer track `index`, as a JSON dict."""
    return json.loads(program.to_json())["doc"]["root"]["tracks"][index]


def _seq_notes(track: dict) -> list:
    """A mixer track's seq notes (unwrapping a reverb chain)."""
    node = track["node"]
    if node["type"] == "chain":
        node = node["stages"][0]
    assert node["type"] == "seq", node["type"]
    return node["notes"]


def _compiled_notes(pattern: "tono.Pattern") -> list:
    """Arrange a pattern on a scratch song and read back (step, pitch, len, gain)."""
    song = tono.Song("ops", tempo=120.0)
    keys = song.track("keys", tono.instruments.piano())
    song.arrange(keys, pattern)
    return [
        (n["step"], n["pitch"], n["len"], n["gain"])
        for n in _seq_notes(_doc_track(song.compile()))
    ]


def test_tempo_map() -> None:
    song = tono.Song("tempo", tempo=120.0)
    keys = song.track("keys", tono.instruments.piano())
    riff = tono.Pattern(bars=2)
    riff.notes(["C4", "E4", "G4", "C5"])
    song.arrange(keys, riff, bars=0)
    song.set_tempo_map([(0, 120.0), (4, 240.0)])
    program = song.compile(sample_rate=48_000)
    # The map reaches the compiled seq and the program meta.
    bundle = json.loads(program.to_json())
    seq = _doc_track(program)["node"]
    assert [(p["at"], p["bpm"]) for p in seq["tempo_map"]] == [
        ({"num": 0, "den": 1}, 120.0),
        ({"num": 4, "den": 1}, 240.0),
    ]
    assert len(bundle["meta"]["tempo_map"]) == 2
    render = program.render()
    assert np.array_equal(render, program.render()), "deterministic with a tempo map"
    assert np.abs(render).max() > 0

    # Every beat form works: int, float, Fraction, (num, den) tuple.
    song.set_tempo_map([(0, 100.0), (1.5, 90.0), (Fraction(5, 2), 80.0), ((3, 1), 70.0)])
    points = json.loads(song.compile().to_json())["meta"]["tempo_map"]
    assert [p["at"] for p in points] == [
        {"num": 0, "den": 1},
        {"num": 3, "den": 2},  # 1.5 is exactly 3/2
        {"num": 5, "den": 2},
        {"num": 3, "den": 1},
    ]
    song.set_tempo_map([])  # clears back to constant tempo
    assert json.loads(song.compile().to_json())["meta"]["tempo_map"] == []

    # Eager validation, before compile: first at 0, ascending, positive bpm.
    for bad in ([(1, 120.0)], [(0, 120.0), (0, 130.0)], [(0, 120.0), (2, 100.0), (1, 90.0)]):
        try:
            song.set_tempo_map(bad)
            raise AssertionError(f"expected ValueError for {bad}")
        except ValueError as exc:
            assert "tempo" in str(exc)
    for bad_bpm in (0.0, -120.0, float("nan"), float("inf")):
        try:
            song.set_tempo_map([(0, bad_bpm)])
            raise AssertionError(f"expected ValueError for bpm {bad_bpm}")
        except ValueError as exc:
            assert "positive" in str(exc)
    # A bad beat type is a TypeError naming the accepted forms.
    try:
        song.set_tempo_map([("soon", 120.0)])
        raise AssertionError("expected TypeError")
    except TypeError as exc:
        assert "Fraction" in str(exc), str(exc)
    # float 0.1 is its binary value, not 1/10 — rejected, pointing at Fraction.
    try:
        song.add_marker("tenth", 0.1)
        raise AssertionError("expected ValueError")
    except ValueError as exc:
        assert "Fraction" in str(exc), str(exc)


def test_meter_map_places_bars_on_the_grid() -> None:
    song = tono.Song("swaying", tempo=120.0)
    keys = song.track("keys", tono.instruments.piano())
    p = tono.Pattern(bars=1)
    p.note("C4")
    song.set_meter_map([(0, 6, 8)])
    song.arrange(keys, p, bars=[0, 1])
    steps = [n["step"] for n in _seq_notes(_doc_track(song.compile()))]
    assert steps == [0, 12], steps  # 6/8: bar 1 = beat 3 = step 12

    # Eager validation mirrors the compiler's rules.
    for bad in ([(1, 4, 4)], [(0, 0, 4)], [(0, 6, 3)], [(0, 4, 4), (2, 3, 4), (1, 6, 8)]):
        try:
            song.set_meter_map(bad)
            raise AssertionError(f"expected ValueError for {bad}")
        except ValueError as exc:
            assert "meter" in str(exc) or "time-signature" in str(exc), str(exc)


def test_pickup_shifts_bar_lines() -> None:
    song = tono.Song("pickup", tempo=120.0)
    keys = song.track("keys", tono.instruments.piano())
    p = tono.Pattern(bars=1)
    p.note("C4")
    song.set_pickup(Fraction(1, 2))  # an eighth-note pickup in 4/4
    song.arrange(keys, p, bars=1)
    steps = [n["step"] for n in _seq_notes(_doc_track(song.compile()))]
    assert steps == [2], steps  # bar 1 starts at beat 1/2 = step 2
    meta = json.loads(song.compile().to_json())["meta"]
    assert meta["pickup"] == {"num": 1, "den": 2}

    song.clear_pickup()
    steps = [n["step"] for n in _seq_notes(_doc_track(song.compile()))]
    assert steps == [16], steps

    # A pickup that pulls bar 1 between grid steps is a ValueError at arrange.
    song.set_pickup(Fraction(1, 3))
    try:
        song.arrange(keys, p, bars=1)
        raise AssertionError("expected ValueError")
    except ValueError as exc:
        assert "grid" in str(exc), str(exc)
    song.clear_pickup()
    try:
        song.set_pickup(-1)
        raise AssertionError("expected ValueError")
    except ValueError as exc:
        assert "negative" in str(exc)


def test_sections_and_markers_reach_the_program() -> None:
    song = tono.Song("structure", tempo=120.0)
    keys = song.track("keys", tono.instruments.piano())
    p = tono.Pattern(bars=1)
    p.note("C4")
    song.arrange(keys, p, bars=range(8))
    song.add_section("verse", 0, 4)
    song.add_section("chorus", 4, 4)
    song.add_marker("drop", 12)
    song.add_marker("fill", Fraction(7, 2))
    meta = json.loads(song.compile().to_json())["meta"]
    assert [(s["name"], s["bar"], s["bars"]) for s in meta["sections"]] == [
        ("verse", 0, 4),
        ("chorus", 4, 4),
    ]
    # Markers sort by position; beats are exact rationals.
    assert [(m["name"], m["at"]) for m in meta["markers"]] == [
        ("fill", {"num": 7, "den": 2}),
        ("drop", {"num": 12, "den": 1}),
    ]
    for bad_call in (
        lambda: song.add_section("", 0, 4),
        lambda: song.add_section("bridge", 8, 0),
        lambda: song.add_marker("", 0),
    ):
        try:
            bad_call()
            raise AssertionError("expected ValueError")
        except ValueError:
            pass


def test_bus_routing_end_to_end() -> None:
    song = tono.Song("buses", tempo=120.0)
    keys = song.track("keys", tono.instruments.piano())
    drums = song.track("drums", tono.instruments.drums())
    p = tono.Pattern(bars=1)
    p.note("C4")
    song.arrange(keys, p)
    song.arrange(drums, p)
    song.add_bus("verb", gain=0.9, effects=[("reverb", {"room": 0.5, "mix": 0.3})])
    song.add_bus("drumbus")
    keys.send("verb", amount=0.4)
    drums.route("drumbus")
    drums.send("verb")  # the default amount, 0.5
    program = song.compile(sample_rate=48_000)
    assert program.stem_routing == {"drums": "drumbus"}
    stems = program.render_stems()
    assert {"keys", "bus:verb", "bus:drumbus"} <= set(stems), stems.keys()
    assert np.abs(stems["keys"]).max() > 0
    assert np.abs(stems["bus:verb"]).max() > 0, "the sends wet the verb bus"
    # The bus insert chain reached the document.
    root = json.loads(program.to_json())["doc"]["root"]
    buses = {b["id"]: b for b in root["buses"]}
    assert buses["verb"]["effects"] == [{"type": "reverb", "room": 0.5, "mix": 0.3}]
    track_sends = {t["id"]: t.get("sends", []) for t in root["tracks"]}
    assert track_sends["keys"] == [{"bus": "verb", "amount": 0.4}]
    assert track_sends["drums"] == [{"bus": "verb", "amount": 0.5}]

    keys.route("verb")
    assert song.compile().stem_routing == {"drums": "drumbus", "keys": "verb"}
    keys.route_master()
    assert song.compile().stem_routing == {"drums": "drumbus"}


def test_routing_and_effect_errors() -> None:
    song = tono.Song("errs", tempo=120.0)
    keys = song.track("keys", tono.instruments.piano())
    p = tono.Pattern(bars=1)
    p.note("C4")
    song.arrange(keys, p)
    song.add_bus("verb")

    # Unknown bus names list the song's buses.
    for call in (lambda: keys.route("nope"), lambda: keys.send("nope")):
        try:
            call()
            raise AssertionError("expected ValueError")
        except ValueError as exc:
            assert "verb" in str(exc), str(exc)

    # A duplicate send target is rejected; clearing re-allows it.
    keys.send("verb", 0.3)
    try:
        keys.send("verb", 0.5)
        raise AssertionError("expected ValueError")
    except ValueError as exc:
        assert "already sends" in str(exc), str(exc)
    keys.clear_sends("verb")
    keys.send("verb", 0.6)
    keys.clear_sends()  # all
    keys.send("verb", 0.2)
    try:
        keys.send("verb", 1.5)
        raise AssertionError("expected ValueError")
    except ValueError as exc:
        assert "[0, 1]" in str(exc)

    # Bad effect types: unknown, and real-but-not-a-processor node types.
    for bad in ("nope", "sine", "seq", "mix", "tracks"):
        try:
            song.add_bus("x", effects=[(bad, {})])
            raise AssertionError(f"expected ValueError for {bad}")
        except ValueError as exc:
            assert "processor" in str(exc), str(exc)
    # A known processor with a mistyped param.
    try:
        song.add_bus("y", effects=[("reverb", {"room": "big"})])
        raise AssertionError("expected ValueError")
    except ValueError as exc:
        assert "reverb" in str(exc)
    # A known processor with an unknown param is caught too.
    try:
        song.add_bus("y", effects=[("reverb", {"rom": 0.5})])
        raise AssertionError("expected ValueError")
    except ValueError as exc:
        assert "rom" in str(exc) and "room" in str(exc), str(exc)
    # Duplicate id, reserved id, non-slug id, out-of-range gain.
    for call in (
        lambda: song.add_bus("verb"),
        lambda: song.add_bus("master"),
        lambda: song.add_bus("My Bus"),
        lambda: song.add_bus("ok", gain=3.0),
    ):
        try:
            call()
            raise AssertionError("expected ValueError")
        except ValueError:
            pass


def test_automation_compiles_to_seconds() -> None:
    song = tono.Song("auto", tempo=120.0)
    keys = song.track("keys", tono.instruments.piano())
    p = tono.Pattern(bars=1)
    p.note("C4")
    song.arrange(keys, p, bars=range(2))
    # 120 BPM: beat 4 = 2.0 s. Handles and names address the track alike.
    song.automate(keys, "gain", [(0.0, 1.0), (4.0, 0.0)])
    song.automate("keys", "pan", [(0.0, -1.0), (2.0, 1.0)], curve="step")
    lanes = _doc_track(song.compile())["automation"]
    by_target = {lane["target"]: lane for lane in lanes}
    assert by_target["gain"]["curve"] == "linear"
    assert [(pt["t"], pt["v"]) for pt in by_target["gain"]["points"]] == [(0.0, 1.0), (2.0, 0.0)]
    assert by_target["pan"]["curve"] == "step"
    assert [(pt["t"], pt["v"]) for pt in by_target["pan"]["points"]] == [(0.0, -1.0), (1.0, 1.0)]

    # Re-automating a target replaces ONLY that lane.
    song.automate(keys, "gain", [(0.0, 0.5)])
    lanes = _doc_track(song.compile())["automation"]
    assert {lane["target"] for lane in lanes} == {"gain", "pan"}
    gain = [lane for lane in lanes if lane["target"] == "gain"][0]
    assert [(pt["t"], pt["v"]) for pt in gain["points"]] == [(0.0, 0.5)]

    # Bad curve, bad target, unknown track (which names the valid tracks).
    for call, needle in (
        (lambda: song.automate(keys, "gain", [(0.0, 1.0)], curve="wobble"), "linear, step, exp"),
        (lambda: song.automate(keys, "resonance", [(0.0, 1.0)]), "gain"),
        (lambda: song.automate("ghost", "gain", [(0.0, 1.0)]), "keys"),
    ):
        try:
            call()
            raise AssertionError("expected ValueError")
        except ValueError as exc:
            assert needle in str(exc), str(exc)


def test_pattern_ops() -> None:
    riff = tono.Pattern(bars=1)
    riff.note("C2", at=0, duration=1)
    riff.note("G2", at=2, duration=0.5, gain=0.8)
    assert repr(riff) == "Pattern(bars=1, notes=2)"

    # transpose shifts and respells; midi: pitches stay midi: pitches.
    assert _compiled_notes(riff.transpose(2)) == [(0, "D2", 4, 1.0), (8, "A2", 2, 0.8)]
    flats = tono.Pattern()
    flats.note("Gb3")
    assert _compiled_notes(flats.transpose(0))[0][1] == "F#3"
    # Ops never mutate their input.
    assert _compiled_notes(riff) == [(0, "C2", 4, 1.0), (8, "G2", 2, 0.8)]

    # reverse mirrors note intervals: [0,4) → [12,16); [8,10) → [6,8).
    assert _compiled_notes(riff.reverse()) == [(6, "G2", 2, 0.8), (12, "C2", 4, 1.0)]

    # repeat offsets each copy; concat/layer/slice rearrange.
    rep = riff.repeat(2)
    assert rep.bars == 2
    assert [s for s, *_ in _compiled_notes(rep)] == [0, 8, 16, 24]
    a = tono.Pattern()
    a.note("C4")
    b = tono.Pattern()
    b.note("E4", at=1)
    assert [s for s, *_ in _compiled_notes(a.concat(b))] == [0, 20]  # b starts after a's bar
    assert a.concat(b).bars == 2
    assert [s for s, *_ in _compiled_notes(a.layer(b))] == [0, 4]
    assert [s for s, *_ in _compiled_notes(riff.slice(8, 8))] == [0]

    # stretch is exact — and refuses off-grid results loudly.
    st = riff.stretch(2, 1)
    assert st.bars == 2
    assert [s for s, *_ in _compiled_notes(st)] == [0, 16]
    try:
        riff.stretch(1, 2)  # a 1-bar pattern can't halve
        raise AssertionError("expected ValueError")
    except ValueError as exc:
        assert "grid" in str(exc), str(exc)

    # rotate wraps around the pattern; quantize snaps starts.
    assert [s for s, *_ in _compiled_notes(riff.rotate(4))] == [4, 12]
    loose = tono.Pattern()
    loose.note("C4", at=0.25)  # step 1
    assert [s for s, *_ in _compiled_notes(loose.quantize(4))] == [0]

    # vel/gate reshape gains and lengths.
    assert _compiled_notes(riff.vel(0.5))[0][3] == 0.5
    assert _compiled_notes(riff.gate(0.5))[0][2] == 2

    # probability/humanize are deterministic per seed.
    assert len(_compiled_notes(riff.probability(1.0, seed=7))) == 2
    assert repr(riff.probability(0.0, seed=7)) == "Pattern(bars=1, notes=0)"
    assert _compiled_notes(riff.probability(0.5, seed=3)) == _compiled_notes(riff.probability(0.5, seed=3))
    assert _compiled_notes(riff.humanize(0.5, 0.2, seed=5)) == _compiled_notes(riff.humanize(0.5, 0.2, seed=5))

    # transpose errors loudly past the MIDI range.
    high = tono.Pattern()
    high.note("midi:127")
    try:
        high.transpose(1)
        raise AssertionError("expected ValueError")
    except ValueError as exc:
        assert "127" in str(exc)


def test_pattern_generators() -> None:
    clave = tono.Pattern.euclidean(3, 8, "midi:36")
    assert [s for s, *_ in _compiled_notes(clave)] == [0, 3, 6]
    assert clave.bars == 1
    assert tono.Pattern.euclidean(3, 8, "midi:36", bars=2).bars == 2
    try:
        tono.Pattern.euclidean(9, 8, "midi:36")  # more pulses than positions
        raise AssertionError("expected ValueError")
    except ValueError:
        pass

    # A triplet across 4 steps: round(i × 4/3) → 0, 1, 3.
    tri = tono.Pattern.tuplet(3, 4, "C4", len=1)
    assert [s for s, *_ in _compiled_notes(tri)] == [0, 1, 3]


def test_harmony() -> None:
    p = tono.Pitch("F#3")
    assert p.midi == 54
    assert p.name == "F#3" and str(p) == "F#3"
    assert repr(p) == "Pitch('F#3')"
    assert tono.Pitch("Gb3") == p, "enharmonics compare equal by midi"
    assert tono.Pitch("midi:54") == p
    assert p.transpose(2) == tono.Pitch("G#3")
    assert p != tono.Pitch("G3")
    for bad_call in (lambda: tono.Pitch("H4"), lambda: tono.Pitch("C"), lambda: p.transpose(100)):
        try:
            bad_call()
            raise AssertionError("expected ValueError")
        except ValueError:
            pass

    key = tono.Key("C major")
    assert key.name == "C major" and repr(key) == "Key('C major')"
    assert key.degree(1) == tono.Pitch("C4")
    assert key.degree(3) == tono.Pitch("E4")
    assert key.degree(8) == tono.Pitch("C5"), "degrees wrap by octave"
    assert key.degree(3, octave=5) == tono.Pitch("E5")
    assert key.contains(tono.Pitch("E4"))
    assert not key.contains(tono.Pitch("F#4"))
    for bad_call in (lambda: tono.Key("C ionian"), lambda: key.degree(0)):
        try:
            bad_call()
            raise AssertionError("expected ValueError")
        except ValueError:
            pass

    cm7 = tono.Chord("Cm7")
    assert cm7.name == "Cm7" and repr(cm7) == "Chord('Cm7')"
    assert cm7.notes() == ["C", "D#", "G", "A#"]
    assert [p.name for p in cm7.pitches()] == ["C4", "D#4", "G4", "A#4"]
    assert [p.name for p in cm7.arp()] == ["C4", "D#4", "G4", "A#4"]
    assert [p.name for p in cm7.invert(1)] == ["D#4", "G4", "A#4", "C5"]
    assert [p.name for p in tono.Chord("C").invert(2, octave=3)] == ["G3", "C4", "E4"]
    try:
        tono.Chord("Csus4")
        raise AssertionError("expected ValueError")
    except ValueError as exc:
        assert "aug" in str(exc), str(exc)


# --- alpha.3: the performance runtime (all headless — CI has no device) ---

# Frame math at 44_100 Hz, 120 BPM, 4/4: one beat = 0.5 s = 22_050 frames,
# one bar = 88_200 frames; beat 4 = bar 1 = 88_200, bar 2 = 176_400.
BLIP_DOC = json.dumps(
    {
        "name": "blip",
        "duration": 0.2,
        "root": {
            "type": "mul",
            "inputs": [
                {"type": "sawtooth", "freq": 880},
                {"type": "env", "a": 0.0, "d": 0.05, "s": 0.0, "r": 0.01},
            ],
        },
    }
)


def performance_program(sample_rate: int = 44_100) -> "tono.Program":
    """A two-track, four-bar program with a section at bar 2 and a marker."""
    song = tono.Song("perf-demo", tempo=120.0)
    bass = song.track("bass", tono.instruments.bass("finger"))
    keys = song.track("keys", tono.instruments.piano())
    riff = tono.Pattern(bars=1)
    riff.note("C2", at=0, duration=1)
    riff.note("G2", at=2, duration=1)
    stab = tono.Pattern(bars=1)
    stab.note("C4", at=0, duration=0.5)
    stab.note("D#4", at=1.5, duration=0.5)
    song.arrange(bass, riff, bars=range(4))
    song.arrange(keys, stab, bars=range(4))
    song.add_section("second", 2, 2)
    song.add_marker("drop", 8)
    return song.compile(sample_rate=sample_rate)


def other_program(sample_rate: int = 44_100) -> "tono.Program":
    """The swap target: a single held note at a different tempo."""
    song = tono.Song("perf-other", tempo=100.0)
    lead = song.track("lead", tono.instruments.piano("bright"))
    phrase = tono.Pattern(bars=1)
    phrase.note("A4", at=0, duration=4)
    song.arrange(lead, phrase)
    return song.compile(sample_rate=sample_rate)


def _fill_chunked(p: "tono.Performance", frames: int, block: int = 512) -> np.ndarray:
    """Render `frames` frames in bounded blocks (the core's gain ramps
    interpolate per rendered slice, so ramp assertions need bounded blocks —
    the same shape as the core's own fill_all test helper)."""
    return np.concatenate([p.fill(min(block, frames - n)) for n in range(0, frames, block)])


def _scripted(p: "tono.Performance") -> None:
    """One scripted session (play, gain at a frame, seek, loop) shared by the
    determinism and capture/replay tests."""
    p.play()
    p.set_gain(0.5, at=tono.at_frame(8_000))
    p.seek_bar(1, at=tono.at_frame(12_000))
    p.set_gain(1.0, at=tono.at_frame(16_000))
    p.set_loop_bars(1, 2, at=tono.at_frame(20_000))


def test_performance_deterministic_and_chunk_invariant() -> None:
    program = performance_program()

    def take(chunks: list) -> np.ndarray:
        with tono.Performance(program, headless=True) as p:
            _scripted(p)
            return np.concatenate([p.fill(n) for n in chunks])

    a, b = take([40_000]), take([40_000])
    assert np.array_equal(a, b), "the same session renders bit-identically twice"
    # Block-boundary invariance (the core's byte-identity promise) holds for a
    # ramp-free session; gain ramps interpolate per rendered slice by design.
    def ramp_free(p: "tono.Performance") -> None:
        p.play()
        p.seek_bar(1, at=tono.at_frame(12_000))
        p.set_loop_bars(1, 2, at=tono.at_frame(20_000))

    def take_ramp_free(chunks: list) -> np.ndarray:
        with tono.Performance(program, headless=True) as p:
            ramp_free(p)
            return np.concatenate([p.fill(n) for n in chunks])

    x, y = take_ramp_free([40_000]), take_ramp_free([512, 333, 40_000 - 845])
    assert np.array_equal(x, y), "the render must not depend on block boundaries"
    assert a.dtype == np.float32 and a.shape == (40_000, 2)
    assert a.flags["C_CONTIGUOUS"]


def test_performance_gain_lands_on_the_frame() -> None:
    program = performance_program()
    at = 10_000
    with tono.Performance(program, headless=True) as plain:
        plain.play()
        ref = _fill_chunked(plain, 30_000)
    assert np.abs(ref[1_000:at]).max() > 0, "the pre region actually sounds"

    with tono.Performance(program, headless=True) as p:
        p.play()
        p.set_gain(0.0, at=tono.at_frame(at))
        got = _fill_chunked(p, 30_000)
    assert np.array_equal(got[:at], ref[:at]), "pre-command audio is untouched"
    tail = at + 1_024  # past the core's gain ramp + per-slice wind-down
    assert np.abs(got[tail:]).max() == 0.0, "gain 0 after the ramp"

    with tono.Performance(program, headless=True) as p:
        p.play()
        p.set_gain(0.5, at=tono.at_frame(at))
        got = _fill_chunked(p, 30_000)
    assert np.array_equal(got[:at], ref[:at])
    assert np.array_equal(got[tail:], ref[tail:] * np.float32(0.5)), "the gain lands exactly"


def test_performance_stinger_onset_lands_on_the_beat() -> None:
    program = performance_program()
    at = 88_200  # beat 4 at 120 BPM, 44.1 kHz
    with tono.Performance(program, headless=True) as plain:
        plain.play()
        ref = plain.fill(at + 4_000)
    with tono.Performance(program, headless=True) as p:
        p.play()
        p.stinger(BLIP_DOC, at=tono.at_beat(4.0))
        got = p.fill(at + 4_000)
        assert p.metrics()["stingers_fired"] == 1
    diff = np.abs(got - ref)
    assert diff[:at].max() == 0.0, "nothing leaks before the stinger's frame"
    assert diff[at : at + 64].max() > 1e-3, "the stinger onset is audible at the beat"


def test_performance_swap_crossfades_and_counts() -> None:
    def take() -> tuple:
        with tono.Performance(performance_program(), headless=True) as p:
            p.play()
            p.swap(other_program(), at=tono.at_frame(20_000))
            out = _fill_chunked(p, 60_000)
            return out, p.metrics()["swaps"]

    (a, swaps_a), (b, _) = take(), take()
    assert np.array_equal(a, b), "the swap is deterministic"
    assert swaps_a == 1
    assert np.abs(a[30_000:]).max() > 0, "the swapped-in program keeps playing"
    # Past the crossfade the output is exactly the new program from its frame 0
    # (bounded blocks: the fade ramps per rendered slice, like the gain ramp).
    with tono.Performance(other_program(), headless=True) as plain:
        plain.play()
        other = _fill_chunked(plain, 10_000)
    assert np.array_equal(a[22_000:30_000], other[2_000:10_000]), "post-fade audio is the new program"

    # A mismatched-rate swap target is rejected at the Python boundary.
    with tono.Performance(performance_program(), headless=True) as p:
        try:
            p.swap(other_program(sample_rate=48_000))
            raise AssertionError("expected ValueError")
        except ValueError as exc:
            assert "48_000" in str(exc).replace(",", "") or "48000" in str(exc), str(exc)


def test_performance_transition_lands_and_unknown_section_raises() -> None:
    program = performance_program()
    with tono.Performance(program, headless=True) as plain:
        plain.play()
        ref = plain.fill(178_400)
    with tono.Performance(program, headless=True) as p:
        p.play()
        p.transition("second", at=tono.at_frame(5_000))
        got = p.fill(12_000)
        # After the seek, the audio is the section's first bar (bar 2 = 176_400).
        assert np.array_equal(got[5_000:7_000], ref[176_400:178_400])
        assert issubclass(tono.UnknownPositionError, tono.PerformanceError)
        assert issubclass(tono.PerformanceError, tono.TonoError)
        try:
            p.transition("nope")
            raise AssertionError("expected UnknownPositionError")
        except tono.UnknownPositionError as exc:
            assert "nope" in str(exc)
        # An unknown position in `at` fails at schedule time too.
        for bad_at in (tono.at_section("nope"), tono.at_marker("nope")):
            try:
                p.play(at=bad_at)
                raise AssertionError("expected UnknownPositionError")
            except tono.UnknownPositionError:
                pass


def test_performance_full_queue_rejects_and_counts() -> None:
    with tono.Performance(performance_program(), headless=True) as p:
        for i in range(4_096):
            p.set_gain(0.5, at=tono.at_frame(1_000_000 + i))
        assert p.queue_depth == 4_096
        assert issubclass(tono.QueueFullError, tono.PerformanceError)
        try:
            p.play()
            raise AssertionError("expected QueueFullError")
        except tono.QueueFullError:
            pass
        m = p.metrics()
        assert m["commands_dropped"] == 1
        assert m["queue_depth_max"] == 4_096


def test_performance_capture_and_replay_reproduces() -> None:
    program = performance_program()
    with tono.Performance(program, headless=True) as a:
        a.capture_start()
        _scripted(a)
        captured = a.capture_stop()
        take_a = a.fill(40_000)
    assert [c["command"] for c in captured] == [
        "Play",
        "SetGain(0.5)",
        "SeekBar(1)",
        "SetGain(1)",
        "SetLoopBars(1, 2)",
    ]
    assert [c["at_frame"] for c in captured] == [0, 8_000, 12_000, 16_000, 20_000]
    assert [c["seq"] for c in captured] == [1, 2, 3, 4, 5]
    # Replaying is re-issuing the captured calls on a fresh performance — the
    # render is deterministic, so the take reproduces bit-exactly.
    with tono.Performance(program, headless=True) as b:
        _scripted(b)
        take_b = b.fill(40_000)
    assert np.array_equal(take_a, take_b), "replay reproduces the take"


def test_performance_snapshot_round_trip() -> None:
    with tono.Performance(performance_program(), headless=True) as p:
        p.play()
        p.fill(10_000)
        snap = p.snapshot()
        assert snap["state"] == "playing"
        assert snap["position_frames"] == 10_000
        assert snap["master_gain"] == 1.0
        assert snap["loop_range"] is None
        take_a = p.fill(4_000)
        p.apply_snapshot(snap)
        assert p.state == "playing" and p.position_frames == 10_000
        take_b = p.fill(4_000)
        assert np.array_equal(take_a, take_b), "the snapshot replays the position"
        # The loop range round-trips through the snapshot dict (frames).
        p.set_loop_bars(1, 2)
        p.fill(1)
        assert p.snapshot()["loop_range"] == (88_200, 176_400)
        p.clear_loop()
        p.fill(1)
        assert p.snapshot()["loop_range"] is None
        # A mistyped or incomplete snapshot is a ValueError, not a crash.
        for bad in ({"position_frames": "soon"}, {"state": "playing"}):
            try:
                p.apply_snapshot(bad)
                raise AssertionError(f"expected ValueError for {bad}")
            except ValueError:
                pass


def test_performance_context_manager_and_transport_properties() -> None:
    with tono.Performance(performance_program(), headless=True) as p:
        assert p.sample_rate == 44_100
        assert p.state == "stopped" and p.position_frames == 0
        p.play()
        out = p.fill(1_000)
        assert out.shape == (1_000, 2)
        assert p.state == "playing" and p.position_frames == 1_000
        assert abs(p.position_beats - 1_000 / 22_050) < 1e-9
        assert abs(p.position_bars - 1_000 / 88_200) < 1e-9
        p.pause()
        p.fill(1)
        assert p.state == "paused"
        pos = p.position_frames
        p.fill(100)
        assert p.position_frames == pos, "paused holds position"
        p.seek_bar(2)
        p.fill(1)
        assert p.position_frames == 176_400
        p.stop()
        p.fill(1)
        assert p.state == "stopped" and p.position_frames == 0
        m = p.metrics()
        assert set(m) == {
            "frames_rendered",
            "commands_executed",
            "commands_dropped",
            "queue_depth_max",
            "swaps",
            "stingers_fired",
            "queue_depth",
        }
        assert m["commands_executed"] == 4  # play, pause, seek_bar, stop
        assert m["frames_rendered"] == 1_103
    # A marker `at` resolves through the program's meta: play starts at beat 8
    # (frame 176_400), so the render is silent until then.
    with tono.Performance(performance_program(), headless=True) as p:
        p.play(at=tono.at_marker("drop"))
        assert p.queue_depth == 1
        got = p.fill(176_400 + 100)
        assert p.state == "playing" and p.position_frames == 100
        assert np.abs(got[:176_400]).max() == 0.0, "silent until the marker"
        assert np.abs(got[176_400:]).max() > 0, "playing after it"


def test_performance_at_and_arg_errors() -> None:
    with tono.Performance(performance_program(), headless=True) as p:
        for bad_at in ("soon", 42, 4.0, object()):
            try:
                p.play(at=bad_at)
                raise AssertionError(f"expected TypeError for {bad_at!r}")
            except TypeError as exc:
                assert "next_bar" in str(exc), str(exc)
        for bad_call in (
            lambda: tono.at_beat(float("nan")),
            lambda: p.seek_beat(float("inf")),
            lambda: p.set_gain(float("nan")),
        ):
            try:
                bad_call()
                raise AssertionError("expected ValueError")
            except ValueError:
                pass
        # Stinger JSON is parsed and validated at the Python boundary.
        try:
            p.stinger("{ not json")
            raise AssertionError("expected ValueError")
        except ValueError:
            pass


def test_performance_live_constructor() -> None:
    program = performance_program()
    # Wrong arg types fail before any device is touched.
    try:
        tono.Performance("not a program")
        raise AssertionError("expected TypeError")
    except TypeError:
        pass
    # Out-of-range and program-mismatched rates are ValueErrors — the program's
    # rate is authoritative (no device needed for these).
    for bad in (123, program.sample_rate + 1):
        try:
            tono.Performance(program, sample_rate=bad)
            raise AssertionError(f"expected ValueError for {bad}")
        except ValueError as exc:
            assert "Hz" in str(exc), str(exc)
    # A live Performance needs an output device; skip gracefully without one
    # (mirrors smoke.py's live-engine check).
    try:
        perf = tono.Performance(program)
    except Exception as exc:
        print(f"live performance skipped: {exc}")
        return
    perf.play()
    time.sleep(0.1)
    assert perf.state == "playing"
    assert perf.position_frames > 0
    assert perf.metrics()["frames_rendered"] > 0
    try:
        perf.fill(100)
        raise AssertionError("expected RuntimeError")
    except RuntimeError:
        pass
    perf.close()
    print("live performance OK")


if __name__ == "__main__":
    test_equivalence_hash_matches_rust()
    test_render_shapes_and_determinism()
    test_bundle_round_trips()
    test_error_paths()
    test_voice_builders_chain_and_pattern_repr()
    test_packaging_and_legacy_surface()
    test_stems_render()
    test_tempo_map()
    test_meter_map_places_bars_on_the_grid()
    test_pickup_shifts_bar_lines()
    test_sections_and_markers_reach_the_program()
    test_bus_routing_end_to_end()
    test_routing_and_effect_errors()
    test_automation_compiles_to_seconds()
    test_pattern_ops()
    test_pattern_generators()
    test_harmony()
    test_performance_deterministic_and_chunk_invariant()
    test_performance_gain_lands_on_the_frame()
    test_performance_stinger_onset_lands_on_the_beat()
    test_performance_swap_crossfades_and_counts()
    test_performance_transition_lands_and_unknown_section_raises()
    test_performance_full_queue_rejects_and_counts()
    test_performance_capture_and_replay_reproduces()
    test_performance_snapshot_round_trip()
    test_performance_context_manager_and_transport_properties()
    test_performance_at_and_arg_errors()
    test_performance_live_constructor()
    print("all typed-API checks passed")
