# Sonarium Cookbook

A sound is one `SoundDoc`:
`{ "name": ..., "duration": secs, "sample_rate": 44100, "seed": 0, "root": <node> }`

`root` is a single node; every node is a mono signal. Multiply a source by an
envelope (`mul`), layer sources with `mix`, and pipe a source through processors
with `chain`. Any numeric param is a constant or a modulator
(`slide` / `lfo` / `arp` / `env`).

## Laser zap — descending square + noise transient
```json
{ "name": "laser_zap", "duration": 0.22, "root": {
  "type": "mix", "inputs": [
    { "type": "mul", "inputs": [
      { "type": "square", "duty": 0.25,
        "freq": { "slide": { "from": 880, "to": 180, "secs": 0.18, "curve": "exp" } } },
      { "type": "env", "a": 0.0, "d": 0.18, "s": 0.0, "r": 0.02, "punch": 0.3 } ] },
    { "type": "mul", "inputs": [
      { "type": "noise" },
      { "type": "env", "a": 0.0, "d": 0.04, "s": 0.0, "r": 0.0 } ] } ] } }
```

## Coin pickup — two ascending blips via arpeggio
```json
{ "name": "coin", "duration": 0.18, "root": {
  "type": "mul", "inputs": [
    { "type": "square", "duty": 0.5, "freq": { "arp": { "steps": [988, 1319], "rate": 14 } } },
    { "type": "env", "a": 0.0, "d": 0.16, "s": 0.0, "r": 0.0, "punch": 0.2 } ] } }
```

## Explosion — noise through a falling lowpass
```json
{ "name": "explosion", "duration": 0.6, "root": {
  "type": "mul", "inputs": [
    { "type": "chain", "stages": [
      { "type": "noise" },
      { "type": "lowpass", "cutoff": { "slide": { "from": 1800, "to": 120, "secs": 0.5, "curve": "exp" } }, "q": 0.7 } ] },
    { "type": "env", "a": 0.0, "d": 0.5, "s": 0.0, "r": 0.1, "punch": 0.6 } ] } }
```

## Reading the feedback

Every render returns **two images** — a spectrogram (freq×time) and a waveform
(amplitude×time) — plus numbers. Read the waveform for the *envelope shape*: a
sharp vertical onset = punchy; a long fade = ringing tail; two humps = a
double-trigger. The numeric `attack_time_ms` / `decay_time_ms` / `onset_count` /
`head_silence_ms` / `tail_silence_ms` quantify exactly that. To converge a sound
toward a reference, call `compare_sounds { a, b }` and drive the reported deltas
(centroid/brightness, LUFS, attack, …) toward zero.

## Tips
- **Punchy/percussive:** `a: 0` (instant attack), short `d`, `s: 0`, add `punch`.
- **Pitch sweeps:** `slide` with `curve: "exp"` reads as natural pitch glide.
- **Brightness:** read `spectral_centroid_hz` from analysis — higher = brighter.
  Tame harshness with a `lowpass`; add bite with a `highpass`.
- **Crunch/lo-fi:** `chain` a source into `bitcrush` (low `bits`) or `downsample`.
- **Vibrato:** put an `lfo` on a source's `freq`. **Tremolo:** `mul` by an `lfo`-driven `gain`... or just an `env`.
- Iterate in small steps: render, read the analysis, change one thing
  (`set_param`), render again. Use `mutate_sound` with a small `amount` to
  nudge a graph toward a variant.

## Music with `seq`

For tunes, write a `seq` instead of gating a drone. Each note has its own pitch,
length (in grid steps), and the shared per-note `env`; gaps are rests; notes can
overlap. `steps_per_beat: 4` = sixteenths.

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

A note's `pitch` accepts a **note name** instead of Hz — `"C4"`, `"F#3"`,
`"Gb5"`, `"midi:60"` (A4 = 440) — so melodies read musically:
```json
{ "type": "seq", "bpm": 120, "steps_per_beat": 4, "wave": "square", "duty": 0.5,
  "env": { "a": 0.005, "d": 0.1, "s": 0.3, "r": 0.05 },
  "notes": [ { "step": 0, "len": 2, "pitch": "C4" }, { "step": 2, "len": 2, "pitch": "E4" },
             { "step": 4, "len": 4, "pitch": "G4" } ] }
```
Note names work for any `freq`/`pitch` (a `sine`/`square`/`super` `freq` too).

Layer voices with `mix` (lead `seq` + bass `seq` + drum `seq`).

**Pitched-drum kick** — a note whose pitch slides down:
```json
{ "type": "seq", "bpm": 120, "wave": "sine",
  "env": { "a": 0.0, "d": 0.18, "s": 0.0, "r": 0.0, "punch": 0.5 },
  "notes": [ { "step": 0, "len": 2, "pitch": { "slide": { "from": 140, "to": 45, "secs": 0.08, "curve": "exp" } } } ] }
```
Use `wave: "noise"` for snares/hats (pitch ignored).

## More timbres

- **PWM lead:** `square` with a modulated `duty` — `{ "lfo": { "shape": "sine", "rate": 5, "depth": 0.3, "center": 0.5 } }`.
- **FM bell / e-piano:** `{ "type": "fm", "freq": 440, "ratio": 3.5, "index": { "slide": { "from": 6, "to": 0, "secs": 0.4 } } }` — higher `ratio`/`index` = more metallic; sliding `index` down gives a struck attack.
- **Warmth / distortion:** `chain` into `drive{amount,shape}` — `tanh` warm, `hard` aggressive, `fold` metallic. Pairs well before a `lowpass`.
- **Fat lead / pad (supersaw):** `{ "type": "super", "wave": "sawtooth", "freq": 220, "voices": 7, "detune_cents": 20 }` — more `voices` / `detune_cents` = wider and thicker. Great through a `lowpass` filter envelope, or as a `mix` layer under a melody.
- **Surgical EQ:** `peak{cutoff,q,gain_db}` boosts/cuts a band (e.g. `+6 dB` at 3 kHz for presence); `lowshelf`/`highshelf{cutoff,gain_db}` tilt the lows/highs; `notch{cutoff,q}` removes a resonance or hum. Read `spectral_centroid_hz`, then EQ to hit the brightness you want.

## Pro techniques

- **Filter envelope (the "pew"/snap):** drive a filter cutoff with an `env` modulator instead of a slide —
  `{ "type": "lowpass", "cutoff": { "env": { "a": 0, "d": 0.12, "s": 0, "r": 0, "from": 4000, "to": 200 } }, "q": 3 }`.
  High `q` + fast decay = laser/zap snap; slow = sweep.
- **Layered impact:** `mix` a low `sine` (slide pitch down) for body + `noise{color:"brown"}` for weight,
  `mul` by a punchy `env`, then `chain` → `lowpass` (env cutoff) → `drive`. Classic hit design.
- **Textures by noise colour:** `white` = hiss/steam, `pink` = wind/surf/rumble, `brown` = distant booms.
- **Metallic / clang:** `fm` with integer-ish `ratio` (3, 3.5) and high `index`, or `ringmod{freq}` on a tone.
- **Width / thickening:** `chorus{rate,depth,mix}` on pads and leads.
- **Glue & loudness:** end a busy chain with `compress{threshold,ratio,attack,release,makeup}`. Watch the
  analysis: keep `true_peak_dbfs` below 0, use `loudness_lufs` to match levels across a set, and read
  `crest_factor_db` (big = punchy transient, small = dense/compressed).
- **Variations (round-robin):** `generate_variants` (or `mutate_sound` with small `amount` 0.1–0.2) spawns N
  subtly different takes of a footstep / impact / pickup so repeats don't sound identical.
- **Stereo (BGM / ambience):** add a top-level `"stereo"` to the doc —
  `{ "mode": "wide", "amount": 0.6 }` for pseudo-stereo width, or
  `{ "mode": "haas", "ms": 12, "pan": -1 }` for precedence widening. SFX usually stay mono (engine spatialises).

## Editing without re-sending the whole graph

A sound persists across restarts and has a stable slug id (from its name, e.g.
`laser_zap`). To change it, you do **not** re-send the whole graph:

1. `describe_sound { id }` → every node's path + type + params, e.g.
   `root.inputs[0].freq`, `root.stages[1].cutoff`.
2. `set_param { id, path, value }` → change one value. `value` is a number, a
   modulator object, or a whole node:
   ```json
   { "id": "laser_zap", "path": "root.inputs[0].inputs[0].freq",
     "value": { "slide": { "from": 880, "to": 140, "secs": 0.18, "curve": "exp" } } }
   ```
3. `edit_sound { id, ops }` → many edits in one re-render (the batch form):
   ```json
   { "id": "impact", "ops": [
     { "op": "set", "path": "root.stages[1].cutoff", "value": 180 },
     { "op": "insert", "path": "root.stages", "index": 2,
       "node": { "type": "compress", "threshold": -14, "ratio": 4, "makeup": 3 } },
     { "op": "remove", "path": "root.stages[0]" } ] }
   ```
   Ops: `set{path,value}` · `insert{path,index?,node}` (into a `chain`'s
   `stages` or a `mix`/`mul`'s `inputs`) · `remove{path,index?}`. Prefer these
   over `refine_sound` (whole-graph replace) for surgical changes.

## Level-matched, click-safe output

Add a top-level `normalize` to gain-match to a loudness target and brick-wall
the true peak (so the file never inter-sample clips):
```json
"normalize": { "target_lufs": -16, "ceiling_dbtp": -1 }
```
Pick **one** `target_lufs` for a whole pack so every sound plays at the same
perceived level (≈ −16 LUFS for SFX, ≈ −14 for music). `export` also takes a
`target_lufs` to write a level-matched asset without touching the stored graph.
`generate_variants` level-matches its round-robin takes automatically.

## Sound packs (the engine-wireable set)

Group related sounds and export them with a manifest a game can read directly:

1. `create_bank { name }` → a pack with a stable id.
2. `add_to_bank { bank_id, sound_id, category?, rr_group? }` — `category`
   (`ui`/`weapon`/`footstep`) lays out subfolders; `rr_group` marks
   interchangeable round-robin takes.
3. `export_bank { bank_id, dest, by_category?, target_lufs? }` → every member
   WAV + a `sounds.json` manifest `{ id, file, category, rr_group, duration_ms,
   sample_rate, channels, lufs, peak_dbfs, true_peak_dbfs }`. `export_all`
   does the same for the whole library.

## Loops, ambience & BGM

For ambience beds, drones, and music that must repeat with no click, set a
top-level `playback`:
```json
"playback": { "mode": "loop", "crossfade_secs": 0.5 }
```
The renderer extracts the loop region (`start_secs`..`end_secs`, default the
whole buffer) and **equal-power crossfades its tail onto its head**, so the
rendered file is a seamless loop body. The exported WAV carries a `smpl` loop
chunk, so Godot / Unity / FMOD loop it at the sample-accurate points with no
manual setup.

- `make_loop { id, crossfade_secs?, start_secs?, end_secs? }` does the same to
  an existing sound and reports the **loop-seam discontinuity in dB** — if it's
  high, raise `crossfade_secs` or match the graph's start/end levels.
- An ambience bed from scratch — slow filter-swept pink noise over a low drone,
  widened and looped:
  ```json
  { "name": "cave_ambience", "duration": 6.0,
    "playback": { "mode": "loop", "crossfade_secs": 0.5 },
    "stereo": { "mode": "wide", "amount": 0.6 },
    "root": { "type": "mix", "inputs": [
      { "type": "chain", "stages": [
        { "type": "noise", "color": "pink" },
        { "type": "lowpass",
          "cutoff": { "lfo": { "shape": "sine", "rate": 0.1, "depth": 250, "center": 600 } } } ] },
      { "type": "chain", "stages": [
        { "type": "sine", "freq": 55 }, { "type": "gain", "amount": 0.4 } ] } ] } }
  ```
- For melodic BGM, build a `seq` (or layer several with `mix`), give the doc a
  `duration` of an exact number of bars, then loop it. Keep the tail tidy
  (notes that ring past the loop point hurt the seam).

## Reproducible sessions

Every mutating tool call is journaled to `session.jsonl` in the working
directory. `save_session { dest }` snapshots that journal; `replay_session
{ path }` re-applies a saved journal — same tool calls, same seeds,
byte-identical audio. Replay requires a **fresh** session (an empty working
directory) and fails otherwise: ids derive from sound names, so replaying
over existing content would silently edit the wrong sounds.
