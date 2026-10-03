# Compose songs

Write tunes with the `seq` sequencer node, then compile a whole piece — tracks, patterns, arrangement — into a hashed, validated **Program**.

## Start with background music

The [background music library](/bgm) has 12 original four-bar themes for menus,
puzzles, exploration, cozy scenes, suspense and action. Each comes in Original,
Calm and Drive arrangements with its own instrumentation and rhythm. Preview
them in your browser, then download a seamless stereo WAV, a SoundDoc recipe,
or the editable Song score. All are MIT licensed and require no sample assets.

```rust
use tono_core::bgm::{BgmSpec, BgmVariant, generate, score};

let spec = BgmSpec::new("lantern-trail", BgmVariant::Calm, 42);
let song = score(&spec)?;       // Editable one-cycle score
let doc = generate(&spec)?;     // Warmed-up, exact four-bar loop
let samples = tono_core::player::render_stereo(&doc);
```

Discover themes through `tono_core::bgm::COMPOSITIONS`. Saved specs pin their
composition revision and seed; save the generated SoundDoc to preserve the
sound across future revisions. Loops include warm-up and crossfade material
outside their playback region, preserving the musical period to the nearest
sample. An exported Song is the editable four-bar score; its ordinary render
is a one-shot. Use the generated SoundDoc for the prepared seamless loop.

To write your own pattern visually, use the [Sound Studio](/create): add an
instrument, enter notes or chords on the grid, and shape the envelope and mix.
Its project exports reopen in the editor and are also valid SoundDocs for the
CLI and existing runtimes.

## Write music with `seq`

<a id="music-with-seq"></a>

For tunes, write a `seq` instead of gating a drone. Each note has its own pitch, length (in grid steps), and the shared per-note `env`; gaps are rests; notes can overlap. `steps_per_beat: 4` = sixteenths.

```json
{ "name": "lead_riff", "duration": 2.0, "root": {
  "type": "seq", "bpm": 120, "steps_per_beat": 4, "wave": "square", "duty": 0.5,
  "env": { "a": 0.005, "d": 0.08, "s": 0.3, "r": 0.04 },
  "notes": [
    { "step": 0, "len": 2, "pitch": 523.25 },
    { "step": 2, "len": 2, "pitch": 659.25 },
    { "step": 4, "len": 4, "pitch": 587.33 },
    { "step": 12, "len": 4, "pitch": 440.00 }
  ] } }
```

A note's `pitch` accepts a **note name** instead of Hz — `"C4"`, `"F#3"`, `"Gb5"`, `"midi:60"` (A4 = 440) — and note names work for any `freq`/`pitch` (a `sine`/`square`/`super` `freq` too):
```json
{ "type": "seq", "bpm": 120, "steps_per_beat": 4, "wave": "square", "duty": 0.5,
  "env": { "a": 0.005, "d": 0.1, "s": 0.3, "r": 0.05 },
  "notes": [ { "step": 0, "len": 2, "pitch": "C4" }, { "step": 2, "len": 2, "pitch": "E4" },
             { "step": 4, "len": 4, "pitch": "G4" } ] }
```

**Pitched-drum kick** — a note whose pitch slides down:
```json
{ "type": "seq", "bpm": 120, "wave": "sine",
  "env": { "a": 0.0, "d": 0.18, "s": 0.0, "r": 0.0, "punch": 0.5 },
  "notes": [ { "step": 0, "len": 2, "pitch": { "slide": { "from": 140, "to": 45, "secs": 0.08, "curve": "exp" } } } ] }
```
Use `wave: "noise"` for snares/hats (pitch ignored). Layer voices with `mix` (lead `seq` + bass `seq` + drum `seq`).

### Seq instruments

Beyond the raw chiptune waves (`square`/`triangle`/`sawtooth`/`sine`/`noise`), `seq` ships a core instrument list — pick one per seq and layer seqs like console tracks:

| wave | sound | notes |
|------|-------|-------|
| `piano` | acoustic piano | detuned string pair, velocity brightness, bass rings/treble dies. Parameter-free. |
| `epiano` | e-piano | soft FM body + metal tine ping; velocity opens the tine. Parameter-free. |
| `organ` | tonewheel organ | drawbar harmonics + attack percussion; sustains while held (`env {s:1}`). |
| `strings` | string ensemble | 3 detuned saws, slow bow swell (~150 ms — write notes slightly early), mellowing lowpass. |
| `brass` | brass section | 2 detuned saws, lowpass swells open over ~70 ms (the blat); velocity opens it further. Parameter-free. |
| `flute` | concert flute | sine + vibrato that fades in (~150 ms), breathy lowpassed noise; velocity adds breath. Parameter-free. |
| `mallet` | marimba | sine fundamental + wooden strike partials dying in tens of ms; velocity brightens. Parameter-free. |
| `bell` | struck bell | inharmonic partials (highs die first, hum rings on) + detuned shimmer twin. Parameter-free. |
| `bass` | fingered bass | filtered saw + sine sub; velocity snaps the filter open. |
| `kit` | drum kit | General MIDI map: pitch picks the drum (see below). |
| `sampler` | **real recorded instruments** | plays any SoundFont: `sf2` path + `sf2_preset` (GM program: 0 grand piano, 32 acoustic bass, 48 strings…); `sf2_bank: 128` = GM drum map. The realism instrument. |
| `cowbell` | pitched cowbell | the phonk lead; also GM 56 in the kit. |
| `fm` | FM mallets/bells | tunable: `fm_ratio` 1 = piano-ish, 3.5 = bell, 14 = tine; `fm_index`/`fm_strike`. |
| `pluck` | plucked string | Karplus-Strong guitar/harp/koto; `pluck_decay` sets ring. |

`piano`, `bass`, and `pluck` also accept optional `piano_*` / `bass_*` / `pluck_*` tone knobs (e.g. `piano_hammer`, `pluck_tone`); every default reproduces the base voice bit-for-bit. `kit` takes a voicing: `kit: "classic"` (default) or `acoustic` / `electronic` / `808`.

**A drum groove** — `kit` reads the note pitch as a GM drum number, not a frequency: `midi:36` kick, `38` snare, `42` closed hat, `46` open hat, `41-50` toms, `49` crash, `51` ride, `39` clap, `56` cowbell:

```json
{ "type": "seq", "bpm": 100, "steps_per_beat": 4, "wave": "kit",
  "env": { "s": 1.0 },
  "notes": [
    { "step": 0,  "len": 2, "pitch": "midi:36" },
    { "step": 4,  "len": 2, "pitch": "midi:38", "gain": 0.9 },
    { "step": 8,  "len": 2, "pitch": "midi:36" },
    { "step": 10, "len": 2, "pitch": "midi:36", "gain": 0.7 },
    { "step": 12, "len": 2, "pitch": "midi:38", "gain": 0.9 },
    { "step": 0,  "len": 1, "pitch": "midi:42", "gain": 0.5 },
    { "step": 2,  "len": 1, "pitch": "midi:42", "gain": 0.4 },
    { "step": 4,  "len": 1, "pitch": "midi:42", "gain": 0.5 },
    { "step": 6,  "len": 1, "pitch": "midi:42", "gain": 0.4 },
    { "step": 8,  "len": 1, "pitch": "midi:42", "gain": 0.5 },
    { "step": 10, "len": 1, "pitch": "midi:42", "gain": 0.4 },
    { "step": 12, "len": 1, "pitch": "midi:42", "gain": 0.5 },
    { "step": 14, "len": 2, "pitch": "midi:46", "gain": 0.6 }
  ] }
```

**A band** is a `tracks` root — the mixing console. Each track has its own `pan` (−1..1, equal-power) and `gain`; `master` is the stereo bus chain. The reverb on the master runs with decorrelated left/right tails, and sampler tracks keep their native recorded stereo:

```json
{ "name": "song", "duration": 4.0, "normalize": { "target_lufs": -14 },
  "root": { "type": "tracks",
    "tracks": [
      { "pan": 0.0, "node": { "type": "seq", "bpm": 100, "wave": "kit", "env": { "s": 1 },
          "notes": [ { "step": 0, "len": 2, "pitch": "midi:36" },
                     { "step": 4, "len": 2, "pitch": "midi:38" } ] } },
      { "pan": 0.0, "gain": 1.1, "node": { "type": "seq", "bpm": 100, "wave": "bass",
          "env": { "a": 0.002, "s": 1, "r": 0.05 },
          "notes": [ { "step": 0, "len": 8, "pitch": "A1" } ] } },
      { "pan": -0.3, "node": { "type": "seq", "bpm": 100, "wave": "epiano",
          "env": { "a": 0.002, "s": 1, "r": 0.15 },
          "notes": [ { "step": 0, "len": 8, "pitch": "A3" },
                     { "step": 0, "len": 8, "pitch": "C#4" } ] } },
      { "pan": 0.35, "node": { "type": "seq", "bpm": 100, "wave": "strings",
          "env": { "a": 0.05, "s": 1, "r": 0.4 },
          "notes": [ { "step": 0, "len": 16, "pitch": "E4" } ] } }
    ],
    "master": [
      { "type": "compress", "threshold": -14, "ratio": 3, "makeup": 2 },
      { "type": "reverb", "room": 0.4, "mix": 0.12 }
    ] } }
```

(`mix` still works for mono layering inside one track.)

**Sampler setup**: download any free General MIDI SoundFont once and point `sf2` at it:
```json
{ "type": "seq", "bpm": 70, "wave": "sampler",
  "sf2": "/Users/you/.tono/sf2/gm.sf2", "sf2_preset": 0,
  "env": { "s": 1, "r": 0.2 },
  "notes": [ { "step": 0, "len": 4, "pitch": "C4" } ] }
```

Groove and glue: every seq takes `swing` (0..1 off-beat delay — ~0.55 is a classic shuffle; off-beats are odd steps, so set `steps_per_beat` to the swung subdivision) and `humanize` (0..1 deterministic timing/velocity jitter — 0.1–0.25 is tasteful). The `duck` processor sidechains anything to a trigger (kick-pumped bass/pads):
```json
{ "type": "chain", "stages": [
  { "type": "seq", "...": "the pad" },
  { "type": "duck", "amount": 0.8, "release": 0.25,
    "trigger": { "type": "seq", "wave": "kit", "...": "the kick pattern" } } ] }
```

Two tunable instruments in detail:

- **`fm`** — a two-operator FM voice struck per note: the modulation index (brightness) starts at `fm_index` and decays over `fm_strike` seconds, like a hammer strike, and louder notes (`gain`) ring brighter. `fm_ratio` picks the timbre family: `1` = e-piano / piano, `2` = hollow / clav, `3.5` = bell, `14` = tine.
  ```json
  { "type": "seq", "bpm": 65, "wave": "fm",
    "fm_ratio": 1.0, "fm_index": 5, "fm_strike": 0.25,
    "env": { "a": 0.002, "d": 1.2, "s": 0.0, "r": 0.3 },
    "notes": [ { "step": 0, "len": 4, "pitch": "A4", "gain": 0.9 },
               { "step": 4, "len": 4, "pitch": "C#5", "gain": 0.7 } ] }
  ```
- **`pluck`** — a Karplus-Strong string: a noise burst rings through a tuned feedback loop whose lowpass damps highs faster than lows, exactly like a real string — guitar, harp, koto. `pluck_decay` (0.8..1) sets ring time; low notes naturally ring longer. Pitch is fixed per note (no glides).
  ```json
  { "type": "seq", "bpm": 90, "wave": "pluck", "pluck_decay": 0.996,
    "env": { "a": 0.0, "d": 0.3, "s": 1.0, "r": 0.2 },
    "notes": [ { "step": 0, "len": 4, "pitch": "E3" },
               { "step": 4, "len": 4, "pitch": "A3" },
               { "step": 8, "len": 8, "pitch": "C#4" } ] }
  ```

Layer them: `fm` melody + soft `triangle` doubling + `pluck` arpeggio is a full band. The pluck's noise burst comes from the doc's `seed`, so takes are reproducible.

## Compile a song to a Program

<a id="songs--from-a-composition-to-a-program"></a>

A whole piece is a **Song**: instrument tracks, reusable patterns, and an arrangement on the bar grid (the `song` module in Rust, `tono.Song` in Python — the same model, the same output).

`Song::compile` produces a **Program**:

- a canonical semantic **hash** over the complete bundle (FNV-1a — identical from Rust or Python for an equivalent song)
- the musical facts: tempo, grid, bars and a track roster in declaration order; track names are persistent identities and inspection indices are zero-based positions
- bounded **resource estimates**: frames, note events, peak voices, memory
- derived **streaming-coverage warnings** — a plain compiled song (a schema-v2 mixer of built-in waves) streams natively with zero warnings; any blocked part is named and falls back to the buffer-backed `Player`

```sh
tono compile SONG.json [-o FILE] [--sample-rate N] [--inspect]
```

- Compilation validates in one pass — every problem at once, each with a stable code, the object path, and the fix. A failing compile exits non-zero, so `tono compile` doubles as a CI gate for song projects.
- `--inspect` prints the machine-readable summary (hash, version pins, roster, estimates, warnings) and writes nothing.
- Without `--inspect` it writes `<name>.program.json` — a versioned bundle that `Program::from_json` / `tono.Program.load` reloads without recompiling. Any revision other than the current format 3 is rejected (`T3001`); a hand-edited one fails its hash check (`T3002`).
- Songs carry their own `engine`/`version` pins like documents, plus an optional song-level `seed`; tracks can be `mute`d or `solo`ed (console semantics: solo mutes every non-solo track; a track that is both stays muted). All of it lands in the Program, so a saved bundle reproduces its audio exactly.

### Move the tempo and meter

```json
"tempo_map": [ { "at": { "num": 0, "den": 1 }, "bpm": 120 },
               { "at": { "num": 16, "den": 1 }, "bpm": 90 } ]
```

- `tempo_map` lists changes at exact beat positions (rationals — tuplets never drift), applied segment-wise, so a note crossing a change keeps its musical length.
- `meter_map` moves time signatures the same way — by bar, with numerator AND denominator (6/8 counts 3 quarter-beats a bar) — and `pickup` gives bar 0 a shorter length (anacrusis).
- A placement that would land between grid steps is a compile error (`T1005`), never a silent rounding; raise `steps_per_beat` or move the placement onto the grid.
- Songs without maps compile byte-identically to before. `sections` (named bar ranges) and `markers` (named beat points) are metadata the Program preserves for the runtime's quantized transitions.

### Automate gain and pan

```json
"automation": [ { "target": "gain", "curve": "exp",
  "points": [ { "at": 0.0, "v": 0.2 }, { "at": 16.0, "v": 0.9 } ] } ]
```

- Song tracks automate `gain`/`pan` in beats, compiled through the tempo map segment-wise.
- Each lane picks a curve: `linear` (default), `step` (hold then jump), or `exp` (geometric between positive endpoints).

### Route buses and sends

```json
"buses": [ { "id": "verb", "gain": 0.8, "effects": [ { "type": "reverb", "room": 0.6, "mix": 1.0 } ] } ]
```

- A tracks root (compiled or hand-written) gains named `buses` — submixes with insert chains and return faders.
- A track routes with `bus`, or feeds a bus post-fader with `sends` (the send taps the ducked, automated signal, so the reverb tail pumps with the mix).

### Export stems

- `tono render --stems DIR` writes every track's positioned contribution and every bus's return as separate stereo WAVs (pre-master-chain); the Python `Program.render_stems()` returns the same as arrays.
- Stems carry their routing: a stem's id is the track id or `bus:<id>`; master-routed stems plus bus returns sum to the exact mix the master chain hears, and a bus-routed track's stem is already inside its bus's stem. Muted tracks render as silent stems.

### Transform patterns in code

- The `song::pattern` module (Rust) and `Pattern` methods (Python) transform patterns purely: `repeat` / `concat` / `layer` / `slice` / `transpose` / `stretch` / `rotate` / `reverse` / `quantize`, `vel` / `gate`, `euclidean` and `tuplet` constructors, and deterministic `probability` / `humanize` seeded per pattern.
- `stretch` is exact or errors (off-grid names the note); `transpose` keeps `midi:N` notes as drums.

### Answer harmony questions

```python
Key("A minor").degree(3)   # the 3rd scale degree
Chord("Cm7").arp()         # root-position voicing
```

The `music` module (Rust) / `tono.Pitch`/`Key`/`Chord` (Python) use a strict spelling grammar — never a silent guess at an ambiguous name.

## Full songs, runnable

Seven complete produced pieces live in the Python examples — clone the repo and run them:

- [`golden_hour.py`](https://github.com/marmikshah/tono/blob/master/crates/tono-py/examples/golden_hour.py) — a produced 16-bar track: swing, humanize, a reverb bus, gain rides; compiles, renders, and bounces a WAV.
- [`fur_elise.py`](https://github.com/marmikshah/tono/blob/master/crates/tono-py/examples/fur_elise.py) — Beethoven's bagatelle on the sampled grand: a true 3/8 meter map with a pickup, a ritardando on the tempo map, per-note dynamics.
- [`monsoon_melody.py`](https://github.com/marmikshah/tono/blob/master/crates/tono-py/examples/monsoon_melody.py) — an original Bollywood-style ballad: chord-symbol voicings from `tono.Chord`, half-bar chord changes, a swung half-time kit, a glockenspiel-shadowed lift, a ritardando outro.
- [`neon_rush.py`](https://github.com/marmikshah/tono/blob/master/crates/tono-py/examples/neon_rush.py) — an original synthwave racer: euclidean tresillo claps, seeded probability hats, a 16th-note octave bass.
- [`noir_lounge.py`](https://github.com/marmikshah/tono/blob/master/crates/tono-py/examples/noir_lounge.py) — an original jazz-noir theme: heavy swing, a walking bass built from `Pitch` arithmetic, rootless Rhodes stabs.
- [`emerald_vale.py`](https://github.com/marmikshah/tono/blob/master/crates/tono-py/examples/emerald_vale.py) — an original fantasy-village pastoral: a true 6/8 meter map, nylon harp-rolls, a rallentando outro.
- [`puzzle_menu.py`](https://github.com/marmikshah/tono/blob/master/crates/tono-py/examples/puzzle_menu.py) — an original puzzle/menu theme: one motif developed by `rotate`/`reverse`/`transpose`, `gate` for the pluck.
