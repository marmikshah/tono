# The tono cookbook — the `SoundDoc` DSL and how to build sounds

`tono` is a deterministic sound engine. You author a sound as a **`SoundDoc`** —
a JSON synthesis graph — and render it. Rendering is a pure function of
`(graph, seed, sample_rate)` → **byte-identical** audio: the same doc always
produces the same WAV, so a `doc.json` *is* the reproducible artifact.

The loop is: write a doc → render it → read the images and stats → change one
thing → re-render.

1. Write a doc:
   `{ "name": ..., "duration": secs, "sample_rate": 44100, "seed": 0, "root": <node> }`.
2. Render it: `tono render doc.json -o out/` writes `out/<name>.wav`,
   `out/<name>.png` (spectrogram), `out/<name>_wave.png` (waveform), and
   `out/<name>.stats.json` (peak / RMS / LUFS / spectral / transient analysis).
3. Look at the two images, read the stats, edit one field in the JSON,
   re-render. Repeat.

**Contents:** [first sounds](#laser-zap--descending-square--noise-transient) ·
[reading the feedback](#reading-the-feedback) · [judging a sound](#judging-a-sound--targets-not-vibes) ·
[music with `seq`](#music-with-seq) · [more timbres](#more-timbres) ·
[pro techniques](#pro-techniques) · [editing by path](#editing-by-path) ·
[layers](#building-sounds-in-layers) · [loudness](#level-matched-click-safe-output) ·
[loops & BGM](#loops-ambience--bgm) · [engine revisions](#engine-revisions-fidelity-vs-byte-stability) ·
[determinism](#determinism)

`root` is a single node; every node is a mono signal. Multiply a source by an
envelope (`mul`), layer sources with `mix`, and pipe a source through processors
with `chain`. Any numeric param is a constant or a modulator
(`slide` / `lfo` / `arp` / `env` / `rand`).

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

`tono render` writes **two images** — a spectrogram (freq×time) and a waveform
(amplitude×time) — plus the numbers in `.stats.json`. The spectrogram's
frequency axis is **logarithmic**, so bass and low-mids (basslines, modal
partials, the body of a sound) get real vertical space instead of being crushed
into a bottom strip. Read the waveform for the *envelope shape*: a sharp
vertical onset = punchy; a long fade = ringing tail; two humps = a
double-trigger. The numeric `attack_time_ms` / `attack_slope_db_per_ms` /
`decay_time_ms` / `onset_count` / `head_silence_ms` / `tail_silence_ms` quantify
exactly that — `attack_slope` is the snappiness readout (big = a click/impact,
small = a swell). Two spectral descriptors round out the picture:
`spectral_flatness` (≈0 tonal/pitched, ≈1 noisy/hissy) and `inharmonicity`
(share of energy *off* the harmonic grid — low for a clean tone, high for noise,
bells/metal, **and aliasing**: it's the meter that shows an anti-aliasing fix
working). To converge a sound toward a reference, render both and compare their
`.stats.json` — drive the deltas (centroid/brightness, LUFS, attack, …) toward
zero.

## Judging a sound — targets, not vibes

Judge a sound by reading its `.stats.json` against concrete targets, so the call
is reproducible rather than a matter of taste. There are two layers of targets.

**The universal ship checklist** (every sound): no clipping — keep
`true_peak_dbfs` below 0; trimmed dead air — small `head_silence_ms` /
`tail_silence_ms`; the right `onset_count` (one hit ⇒ 1; a double-trigger shows
up as 2); and a clean loop seam for anything that repeats.

**Per-archetype targets** — what "good" means for a kind of sound, judged mostly
on attack, spectral centroid, crest, and duration:

| archetype | character to hit |
|-----------|------------------|
| `laser` | short, bright, falling, very punchy |
| `coin` | two bright blips, moderate punch |
| `jump` | short rising sweep, fast gate |
| `impact` | low-centred body with a ring tail |
| `ui` | tiny, bright, instant |
| `ambience` | sustained, dark, low crest, looping |
| `bgm` | a mixed musical loop |

For a `laser`, aim for a crest of at least ~12 dB and a `spectral_centroid_hz`
in the 2–8 kHz range. If the stats read crest 7 dB, add `punch` and shorten the
attack; if the centroid reads 1200 Hz, raise a filter cutoff or pick a brighter
wave. Drive a **polish loop**: read the stats → apply the single highest-impact
fix by editing one field → re-render → if it regressed, revert that edit →
repeat until the targets are met. Don't chase a deviation the sound's character
justifies (a bell's long tail, a gusting wind's crest) — stop at the targets,
not past them. (`tono_core::review` grades a doc against an archetype
programmatically if you want the checklist automated in Rust.)

## Tips
- **Punchy/percussive:** `a: 0` (instant attack), short `d`, `s: 0`, add `punch`.
- **Pitch sweeps:** `slide` with `curve: "exp"` reads as natural pitch glide.
- **Brightness:** read `spectral_centroid_hz` from the stats — higher = brighter.
  Tame harshness with a `lowpass`; add bite with a `highpass`.
- **Crunch/lo-fi:** `chain` a source into `bitcrush` (low `bits`) or `downsample`.
- **Vibrato:** put an `lfo` on a source's `freq`. **Tremolo:** `mul` by an `lfo`-driven `gain` … or just an `env`.
- Iterate in small steps: render, read the stats, change one field, render again.
  `tono_core::vary::mutate(doc, amount, seed)` (a Rust API) nudges a graph toward
  a variant with a small `amount`.

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

### Seq instruments

Beyond the raw chiptune waves (`square`/`triangle`/`sawtooth`/`sine`/`noise`),
`seq` ships a core instrument list — pick one per seq and layer seqs like
console tracks:

| wave | sound | notes |
|------|-------|-------|
| `piano` | acoustic piano | detuned string pair, velocity brightness, bass rings/treble dies. Parameter-free. |
| `epiano` | Rhodes e-piano | soft FM body + metal tine ping; velocity opens the tine. Parameter-free. |
| `organ` | tonewheel organ | drawbar harmonics + attack percussion; sustains while held (`env {s:1}`). |
| `strings` | string ensemble | 3 detuned saws, slow bow swell (~150 ms — write notes slightly early), mellowing lowpass. |
| `bass` | fingered bass | filtered saw + sine sub; velocity snaps the filter open. |
| `kit` | drum kit | General MIDI map: pitch picks the drum (see below). |
| `sampler` | **real recorded instruments** | plays any SoundFont: `sf2` path + `sf2_preset` (GM program: 0 grand piano, 32 acoustic bass, 48 strings…); `sf2_bank: 128` = GM drum map. The realism instrument. |
| `cowbell` | pitched cowbell | the phonk lead; also GM 56 in the kit. |
| `fm` | FM mallets/bells | tunable: `fm_ratio` 1 = piano-ish, 3.5 = bell, 14 = tine; `fm_index`/`fm_strike`. |
| `pluck` | plucked string | Karplus-Strong guitar/harp/koto; `pluck_decay` sets ring. |

**A drum groove** — `kit` reads the note pitch as a GM drum number, not a
frequency: `midi:36` kick, `38` snare, `42` closed hat, `46` open hat,
`41-50` toms, `49` crash, `51` ride, `39` clap:

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

**A band** is a `tracks` root — the mixing console. Each track has its own
`pan` (−1..1, equal-power) and `gain`; `master` is the stereo bus chain. The
reverb on the master runs with decorrelated left/right tails, and sampler
tracks keep their native recorded stereo:

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

**Sampler setup**: download any General MIDI SoundFont once (e.g. FluidR3 GM
or GeneralUser GS, both free) and point `sf2` at it:
```json
{ "type": "seq", "bpm": 70, "wave": "sampler",
  "sf2": "/Users/you/.tono/sf2/gm.sf2", "sf2_preset": 0,
  "env": { "s": 1, "r": 0.2 },
  "notes": [ { "step": 0, "len": 4, "pitch": "C4" } ] }
```
Groove: every seq takes `swing` (0..1 off-beat delay — shuffle) and
`humanize` (0..1 deterministic timing/velocity jitter). Glue: the `duck`
processor sidechains anything to a trigger (kick-pumped bass/pads):
```json
{ "type": "chain", "stages": [
  { "type": "seq", "...": "the pad" },
  { "type": "duck", "amount": 0.8, "release": 0.25,
    "trigger": { "type": "seq", "wave": "kit", "...": "the kick pattern" } } ] }
```

Two tunable instruments in detail:

- **`fm`** — a two-operator FM voice struck per note: the modulation index
  (brightness) starts at `fm_index` and decays over `fm_strike` seconds, like
  a hammer strike, and louder notes (`gain`) ring brighter. `fm_ratio` picks
  the timbre family: `1` = e-piano / piano, `2` = hollow / clav, `3.5` = bell,
  `14` = tine.
  ```json
  { "type": "seq", "bpm": 65, "wave": "fm",
    "fm_ratio": 1.0, "fm_index": 5, "fm_strike": 0.25,
    "env": { "a": 0.002, "d": 1.2, "s": 0.0, "r": 0.3 },
    "notes": [ { "step": 0, "len": 4, "pitch": "A4", "gain": 0.9 },
               { "step": 4, "len": 4, "pitch": "C#5", "gain": 0.7 } ] }
  ```
- **`pluck`** — a Karplus-Strong string: a noise burst rings through a tuned
  feedback loop whose lowpass damps highs faster than lows, exactly like a
  real string — guitar, harp, koto. `pluck_decay` (0.8..1) sets ring time;
  low notes naturally ring longer. Pitch is fixed per note (no glides).
  ```json
  { "type": "seq", "bpm": 90, "wave": "pluck", "pluck_decay": 0.996,
    "env": { "a": 0.0, "d": 0.3, "s": 1.0, "r": 0.2 },
    "notes": [ { "step": 0, "len": 4, "pitch": "E3" },
               { "step": 4, "len": 4, "pitch": "A3" },
               { "step": 8, "len": 8, "pitch": "C#4" } ] }
  ```

Layer them: `fm` melody + soft `triangle` doubling + `pluck` arpeggio is a
full band. The pluck's noise burst comes from the doc's `seed`, so takes are
reproducible.

## More timbres

- **PWM lead:** `square` with a modulated `duty` — `{ "lfo": { "shape": "sine", "rate": 5, "depth": 0.3, "center": 0.5 } }`.
- **FM bell / e-piano:** `{ "type": "fm", "freq": 440, "ratio": 3.5, "index": { "slide": { "from": 6, "to": 0, "secs": 0.4 } } }` — higher `ratio`/`index` = more metallic; sliding `index` down gives a struck attack.
- **Warmth / distortion:** `chain` into `drive{amount,shape}` — `tanh` warm, `hard` aggressive, `fold` metallic. Pairs well before a `lowpass`. On `engine: 1` documents the shaper is anti-aliased (ADAA) so hard/fold stay clean instead of spraying inharmonic foldback; set `"aa": false` on the node to hear the raw aliasing curve.
- **Struck bodies (bell / glass / metal / coin / UI ping):** the **exciter → resonator** pair — an `impact` into a `modal` bank. `chain[ {type:impact, hardness:0.85}, {type:modal, modes:[{freq,decay,gain}, …]} ]`. Each mode is a damped sine; near-harmonic ratios + a long fundamental = bell, off-harmonic ratios = metal, all-short decays = a glass/UI tick. The hammer's `hardness` sets how far up the bank it reaches; `velocity` its energy. Oscillators can't voice these cleanly — modes can.
- **Fat lead / pad (supersaw):** `{ "type": "super", "wave": "sawtooth", "freq": 220, "voices": 7, "detune_cents": 20 }` — more `voices` / `detune_cents` = wider and thicker. Great through a `lowpass` filter envelope, or as a `mix` layer under a melody.
- **Surgical EQ:** `peak{cutoff,q,gain_db}` boosts/cuts a band (e.g. `+6 dB` at 3 kHz for presence); `lowshelf`/`highshelf{cutoff,gain_db}` tilt the lows/highs; `notch{cutoff,q}` removes a resonance or hum. Read `spectral_centroid_hz`, then EQ to hit the brightness you want.

## Pro techniques

- **Filter envelope (the "pew"/snap):** drive a filter cutoff with an `env` modulator instead of a slide —
  `{ "type": "lowpass", "cutoff": { "env": { "a": 0, "d": 0.12, "s": 0, "r": 0, "from": 4000, "to": 200 } }, "q": 3 }`.
  High `q` + fast decay = laser/zap snap; slow = sweep.
- **Layered impact:** `mix` a low `sine` (slide pitch down) for body + `noise{color:"brown"}` for weight,
  `mul` by a punchy `env`, then `chain` → `lowpass` (env cutoff) → `drive`. Classic hit design.
- **Textures by noise colour:** `white` = hiss/steam, `pink` = wind/surf/rumble, `brown` = distant booms.
- **Crackle / sparse events (`dust`):** `{ "type": "dust", "density": 80, "decay": 0.025 }` is a Poisson click train — `density` grains/sec, each ringing `decay` seconds (0 = bare impulses). Fire crackle, rain, geiger ticks, sparks, debris. Band-shape it through a `bandpass`/`highpass`, or feed a `modal` for pitched debris.
- **Organic motion (`rand`):** a random-walk modulator — `{ "rand": { "from": 250, "to": 1500, "rate": 0.7, "seed": 1 } }` — drifts non-periodically between `from` and `to`, `rate` new targets/sec. The gusting the periodic `lfo` can't do: wind (on a lowpass `cutoff`), fire flicker (on a `gain`), drifting detune. Give two `rand`s different `seed`s to decorrelate them; the walk is deterministic and edit-stable (seeded from its own fields, never shifts when siblings change).
- **Metallic / clang:** a `modal` bank with off-harmonic mode ratios excited by a hard `impact` (the physical way — see "Struck bodies" above); or, cheaper, `fm` with integer-ish `ratio` (3, 3.5) and high `index`, or `ringmod{freq}` on a tone.
- **Tuning a modal bank:** address one partial at a time — set the field at
  `root.stages[1].modes[0].freq` to 540 (each mode is its own node with its own
  path). Stretch every `decay` for a cathedral bell, shrink them for a desk
  bell; raise `hardness` toward 1 to wake the upper modes. `tono_core::vary::mutate`
  then gives a non-repeating round-robin of hits.
- **Width / thickening:** `chorus{rate,depth,mix}` on pads and leads.
- **Glue & loudness:** end a busy chain with `compress{threshold,ratio,attack,release,makeup}`. Watch the
  stats: keep `true_peak_dbfs` below 0, use `loudness_lufs` to match levels across a set, and read
  `crest_factor_db` (big = punchy transient, small = dense/compressed).
- **Variations (round-robin):** `tono_core::vary::mutate(doc, amount, seed)` — a
  Rust API — with a small `amount` (0.1–0.2) spawns N subtly different takes of a
  footstep / impact / pickup so repeats don't sound identical.
- **Stereo (BGM / ambience):** add a top-level `"stereo"` to the doc —
  `{ "mode": "wide", "amount": 0.6 }` for pseudo-stereo width, or
  `{ "mode": "haas", "ms": 12, "pan": -1 }` for precedence widening. SFX usually stay mono (engine spatialises).

## Editing by path

Every node has a **path**: `root.inputs[0].freq`, `root.stages[1].cutoff`,
`root.stages[1].modes[0].freq`. To change a sound you edit that field in the JSON
and re-render — no need to rewrite the whole graph. Paths index into a `chain`'s
`stages`, a `mix`/`mul`'s `inputs`, a `seq`'s `notes`, and a `tracks`' `tracks`.
A field's value can be a number, a modulator object, or a whole node — e.g. set
`root.inputs[0].inputs[0].freq` to:
```json
{ "slide": { "from": 880, "to": 140, "secs": 0.18, "curve": "exp" } }
```
For programmatic editing there's a small Rust API in `tono_core::edit`:
`describe(doc)` returns the path → type → params map; `apply_ops(doc, ops)`
applies a batch of `set{path,value}` · `insert{path,index?,node}` (into a
`chain`'s `stages` or a `mix`/`mul`'s `inputs`) · `remove{path,index?}` ops in
one pass; and `morph(a, b, t)` blends two docs.

## Building sounds in layers

Pro SFX are stacks: a transient (the click that says "now"), a body (the
identity), a tail (the space). Build them as a **`tracks` root** — the mixer —
with one track per component. Each track carries a stable `id`, its own `pan`
(−1..1, equal-power), a `gain` (0..2, 1 = unity), a start offset `at` (seconds —
a tail 20 ms late is `at: 0.02`, a pre-click 5 ms early against a body at
`at: 0.005`), and a `mute` flag. `mute` is rendered state, not a monitoring
convenience: exports ship without muted tracks.

A disciplined SFX skeleton is four band-split layers — `sub` / `body` / `top` /
`transient` — each a `chain` of its source into a band-splitting filter and a
one-shot envelope, with a starting `gain`. Fill in the real source per role
(an `fm` or `super` for the body, `noise` → `highpass` for the top, an `impact`
for the transient), then rebalance by reading each layer's contribution.

Balance with the per-layer stats every render returns: `.stats.json` carries a
`layers` array, one entry per track — `{ id, peak_dbfs, rms_dbfs, share, muted }`,
where `share` is that layer's percentage of the pre-master energy (e.g.
`crack 38% • peak −8.1 dBFS | body 52% … | tail 10%`). Nudge a track's `gain`
until the split reads right, and edit inside a layer with paths into its `node`
(e.g. `root.tracks[0].node.env.d`).

**One layer per thing you'd fade, pan, time-shift, or analyze separately** — an
instrument in a song, a component in an SFX. Use `mix` only for sub-signals that
share one envelope/filter; never one track holding a mix of seqs (it makes the
per-layer stats useless).

Layers are independent by construction: each track's noise is drawn from a
deterministic RNG stream keyed by its `id`, so muting, removing, duplicating, or
editing one track never changes a sibling's noise grains. Duplicating a track
under a new `id` is a built-in variation — the copy re-grains its noise
deterministically from the new id.

## Level-matched, click-safe output

Add a top-level `normalize` to gain-match to a loudness target and brick-wall
the true peak (so the file never inter-sample clips):
```json
"normalize": { "target_lufs": -16, "ceiling_dbtp": -1 }
```
Pick **one** `target_lufs` for a whole set so every sound plays at the same
perceived level (≈ −16 LUFS for SFX, ≈ −14 for music). To ship a set, render
each doc into the same output folder with the same `target_lufs`; each
`.stats.json` reports the resulting `loudness_lufs` / `true_peak_dbfs` so you
can confirm they match.

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

- Watch the loop seam: if it clicks, raise `crossfade_secs` or match the graph's
  start/end levels.
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

## Engine revisions (fidelity vs. byte-stability)

A document carries two independent version numbers. `version` is the **schema**
version (document structure). `engine` is the **DSP-kernel** revision — which
audio kernels render it. They are split so a fidelity upgrade never changes the
bytes of an older sound: a document with `engine` omitted renders under the
original kernels (byte-for-byte forever); new documents are stamped with the
current engine and get the upgrades. The revisions so far:

- **1** — anti-aliased `drive` (ADAA).
- **2** — per-node structurally-seeded RNG for `noise`/`dust` (decorrelated
  siblings; byte-identical streaming randomness).
- **3** — the inharmonic additive `piano` voice (stretched partials,
  per-partial decay, hammer spectrum, detuned unison pair).
- **4** — corrected mixer output stage (joint stereo loudness normalization,
  gated BS.1770, oversampled true-peak) and per-note humanize jitter.

To modernise an existing sound, set `"engine": 4` (the current revision — its
output will change; that's the point); to keep a legacy sound bit-exact, leave
`engine` off.

## Determinism

Rendering is a pure function of `(graph, seed, sample_rate)` — a doc renders
**byte-identical** every time. (Today the guarantee is per platform: the DSP
calls the OS math library for `sin`/`exp`/`powf`, whose last bits differ between
macOS-arm64 and linux-x86_64. Integer-RNG, PolyBLEP, and rational-filter content
is identical everywhere; a future engine revision with deterministic
transcendental kernels makes it truly cross-platform.) The doc's top-level `seed` drives
every noise source, `dust` train, and Karplus-Strong pluck burst, so takes are
reproducible; change `seed` for a different-but-equivalent roll. Because the doc
*is* the artifact, version your `.json` files and you can always reproduce the
exact WAV — no separate session log needed.
