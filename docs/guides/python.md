# Python

The deterministic tono engine plus its live runtime, from Python: typed songs, numpy renders, and a speaker-owning engine — the `tono` module from [crates/tono-py](https://github.com/marmikshah/tono/tree/master/crates/tono-py).

## Install

Build from a repository checkout in a virtual environment. Requires the
workspace's Rust toolchain and CPython 3.9+. The manual Wheels workflow
produces abi3 artifacts for Linux x86_64.

```sh
python -m pip install maturin numpy
maturin develop -m crates/tono-py/Cargo.toml              # the `tono` module in your env
python3 crates/tono-py/tests/smoke.py                     # the determinism smoke test
maturin build --release -m crates/tono-py/Cargo.toml      # abi3 wheel → target/wheels/
```

## Compile a song

```python
import tono

song = tono.Song("night-drive", tempo=122.0)
bass = song.track("bass", tono.instruments.bass("finger"))
riff = tono.Pattern(bars=1)
riff.notes(["C2", "C2", "Eb2", "G2"], durations=0.5)
song.arrange(bass, riff, bars=range(4))

program = song.compile(sample_rate=48000)   # tono.CompileError carries .diagnostics
audio = program.render()                    # np.float32, shape (frames, 2), L/R
```

Typed `Song` / `Pattern` / `Program` wrap the native Rust model: no JSON,
`py.typed` stubs in the wheel, and the same canonical program hash an
equivalent Rust song compiles to. `program.save(path)` /
`tono.Program.load(path)` ship the hashed bundle.

## Run it live

```python
with tono.Performance(program, headless=True) as perf:   # or headless=False for speakers
    perf.play()
    perf.set_gain(0.8, at=tono.next_bar())
    perf.transition("chorus", at=tono.next_bar())        # a named section
    audio = perf.fill(program.sample_rate * 10)          # stereo (frames, 2), float32
    print(perf.metrics())                                # frames, commands, queue depth, …
```

Commands schedule at frames, beats, bars, markers, sections, `tono.next_beat()`,
or `tono.next_bar()` — and execute at exact frames, never waiting on Python to
wake up. Seeks, loops, crossfaded swaps, stingers, capture/replay, and
snapshots work the same in both modes.

## Runnable examples

All in [`crates/tono-py/examples/`](https://github.com/marmikshah/tono/tree/master/crates/tono-py/examples):

- [`night_drive.py`](https://github.com/marmikshah/tono/blob/master/crates/tono-py/examples/night_drive.py) — the typed API end to end: compose, compile, render, and run a program live with scheduled commands.
- [`golden_hour.py`](https://github.com/marmikshah/tono/blob/master/crates/tono-py/examples/golden_hour.py) — a produced 16-bar track; compiles, renders, and bounces `golden_hour.wav` you can play.
- [`fur_elise.py`](https://github.com/marmikshah/tono/blob/master/crates/tono-py/examples/fur_elise.py) — Beethoven's bagatelle on the sampled grand: 3/8 meter map with a pickup, a tempo-map ritardando, per-note dynamics.
- [`monsoon_melody.py`](https://github.com/marmikshah/tono/blob/master/crates/tono-py/examples/monsoon_melody.py) — an original Bollywood-style ballad: flute over nylon-guitar arpeggios, a swung half-time kit, a glockenspiel-shadowed lift.
- [`neon_rush.py`](https://github.com/marmikshah/tono/blob/master/crates/tono-py/examples/neon_rush.py) — an original synthwave racer: euclidean claps, probability hats, a 16th-note octave bass.
- [`noir_lounge.py`](https://github.com/marmikshah/tono/blob/master/crates/tono-py/examples/noir_lounge.py) — an original jazz-noir theme: heavy swing, walking bass, rootless Rhodes stabs.
- [`emerald_vale.py`](https://github.com/marmikshah/tono/blob/master/crates/tono-py/examples/emerald_vale.py) — an original fantasy-village pastoral: a true 6/8 meter map, nylon harp-rolls.
- [`puzzle_menu.py`](https://github.com/marmikshah/tono/blob/master/crates/tono-py/examples/puzzle_menu.py) — an original puzzle/menu theme built by pattern transforms.
- [`live_pygame.py`](https://github.com/marmikshah/tono/blob/master/crates/tono-py/examples/live_pygame.py) — live procedural audio for a Python game loop in ~10 lines.
- [`render_numpy.py`](https://github.com/marmikshah/tono/blob/master/crates/tono-py/examples/render_numpy.py) — the pull API: render to a numpy array and use it anywhere.
