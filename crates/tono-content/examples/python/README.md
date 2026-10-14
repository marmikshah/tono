# Python reference scores

Seven complete compositions written against the typed Python API in
`crates/tono-py`. They are reference scores rather than API tours: each one
composes a full piece end to end, compiles it, and renders it. The site's
[composition catalog](https://marmikshah.github.io/tono/) links to them as the
source of each track.

| Script | Piece |
| --- | --- |
| `emerald_vale.py` | Fantasy-village pastoral in G major, 6/8 |
| `fur_elise.py` | Beethoven's "Für Elise" opening on the sampled grand |
| `golden_hour.py` | Produced 16-bar boom-bap piece with a vibraphone melody |
| `monsoon_melody.py` | Bollywood-style romantic ballad in D minor |
| `neon_rush.py` | Synthwave/outrun racing piece in C minor |
| `noir_lounge.py` | Jazz-noir lounge piece in D minor |
| `puzzle_menu.py` | Quirky puzzle/menu theme in C major |

Build the bindings once, then run a score:

```sh
python -m pip install maturin numpy
maturin develop -m crates/tono-py/Cargo.toml
python crates/tono-content/examples/python/<name>.py
```

The shorter tours of the API itself (`night_drive.py`, `render_numpy.py`,
`live_pygame.py`) stay in `crates/tono-py/examples`.
