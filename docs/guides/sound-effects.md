# Design sound effects

Author a sound effect as a **SoundDoc** — a JSON synthesis graph — and iterate: render, read the feedback, change one thing, re-render. (The full node vocabulary is [the reference](/reference/sounddoc).)

## Cook your first SFX

**Laser zap** — descending square + noise transient:
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

**Coin pickup** — two ascending blips via arpeggio:
```json
{ "name": "coin", "duration": 0.18, "root": {
  "type": "mul", "inputs": [
    { "type": "square", "duty": 0.5, "freq": { "arp": { "steps": [988, 1319], "rate": 14 } } },
    { "type": "env", "a": 0.0, "d": 0.16, "s": 0.0, "r": 0.0, "punch": 0.2 } ] } }
```

**Explosion** — noise through a falling lowpass:
```json
{ "name": "explosion", "duration": 0.6, "root": {
  "type": "mul", "inputs": [
    { "type": "chain", "stages": [
      { "type": "noise" },
      { "type": "lowpass", "cutoff": { "slide": { "from": 1800, "to": 120, "secs": 0.5, "curve": "exp" } }, "q": 0.7 } ] },
    { "type": "env", "a": 0.0, "d": 0.5, "s": 0.0, "r": 0.1, "punch": 0.6 } ] } }
```

Design tips:

- **Punchy/percussive:** `a: 0` (instant attack), short `d`, `s: 0`, add `punch`.
- **Brightness:** read `spectral_centroid_hz` — higher = brighter. Tame harshness with a `lowpass`; add bite with a `highpass`.
- **Crunch/lo-fi:** `chain` a source into `bitcrush` (low `bits`) or `downsample`.
- **Vibrato:** an `lfo` on a source's `freq`. **Tremolo:** `mul` by an `lfo`-driven `gain` … or just an `env`.

## Read the feedback

`tono render` writes two images plus the numbers in `.stats.json`. Read them against each other:

| output | read it as |
|--------|------------|
| spectrogram `.png` | **log frequency axis** — bass and low-mids (basslines, modal partials, the body) get real vertical space |
| waveform `_wave.png` | envelope shape: sharp vertical onset = punchy; long fade = ringing tail; two humps = double-trigger |
| `attack_time_ms` / `attack_slope_db_per_ms` | onset speed / snappiness — big slope = a click/impact, small = a swell |
| `decay_time_ms` | tail length |
| `onset_count` | trigger count (one hit ⇒ 1; a double-trigger ⇒ 2) |
| `head_silence_ms` / `tail_silence_ms` | dead air to trim |
| `spectral_centroid_hz` | brightness |
| `spectral_flatness` | ≈0 tonal/pitched … ≈1 noisy/hissy |
| `inharmonicity` | energy *off* the harmonic grid — low for a clean tone, high for noise, bells/metal, **and aliasing** (the meter that shows an anti-aliasing fix working) |
| `true_peak_dbfs` | inter-sample peak; keep below 0 |
| `loudness_lufs` | perceived level |
| `crest_factor_db` | big = punchy transient, small = dense/compressed |

To converge a sound toward a reference, render both and compare their `.stats.json` — drive the deltas (centroid/brightness, LUFS, attack, …) toward zero.

## Judge a sound by targets

Judge against concrete targets so the call is reproducible, not taste.

**The universal ship checklist** (every sound):

- no clipping — `true_peak_dbfs` below 0
- trimmed dead air — small `head_silence_ms` / `tail_silence_ms`
- the right `onset_count` (one hit ⇒ 1)
- a clean loop seam for anything that repeats

**Per-archetype targets** — judged mostly on attack, spectral centroid, crest, and duration:

| archetype | character to hit |
|-----------|------------------|
| `laser` | short, bright, falling, very punchy |
| `coin` | two bright blips, moderate punch |
| `jump` | short rising sweep, fast gate |
| `impact` | low-centred body with a ring tail |
| `ui` | tiny, bright, instant |
| `footstep` | a very short, low-mid thud/tap |
| `powerup` | a short bright rising flourish |
| `ambience` | sustained, dark, low crest, looping |
| `bgm` | a mixed musical loop |

For a `laser`, aim for a crest of at least ~12 dB and a `spectral_centroid_hz` in the 2–8 kHz range. If the stats read crest 7 dB, add `punch` and shorten the attack; if the centroid reads 1200 Hz, raise a filter cutoff or pick a brighter wave.

The polish loop: read the stats → apply the single highest-impact fix by editing one field → re-render → if it regressed, revert that edit → repeat until the targets are met. Don't chase a deviation the sound's character justifies (a bell's long tail, a gusting wind's crest) — stop at the targets, not past them.

```sh
tono review doc.json --archetype laser
```

`tono review` runs the whole checklist for you — every finding prints the measured value, the target, and the fix to try, and the command exits non-zero on a FAIL grade, so it doubles as a ship gate in CI. (The same grading is `tono_core::review` in Rust.)

## Match a reference sound

Bring a WAV you like and measure a doc against it:

```sh
tono match REF.wav doc.json      # how far off, metric by metric
tono fit REF.wav doc.json        # close the gap automatically
```

- `tono match` prints the reference-vs-candidate table (brightness, loudness, envelope, duration), worst offenders first, with an overall score in tolerance units.
- `tono fit` hill-climbs that score: each round applies a seeded `vary::mutate` perturbation to the incumbent, keeps it when it scores closer, and halves the step size when the search stalls.
- The search is deterministic — same reference, doc, `--rounds`/`--amount`/`--seed`, same result — and writes the best doc to `<doc>.fit.json` (or `-o`) with its final match table.
- Start broad (`--amount 0.4 --rounds 64`), then re-fit the output with a small `--amount` to polish.

## Design more timbres

- **PWM lead:** `square` with a modulated `duty` — `{ "lfo": { "shape": "sine", "rate": 5, "depth": 0.3, "center": 0.5 } }`.
- **FM bell / e-piano:** `{ "type": "fm", "freq": 440, "ratio": 3.5, "index": { "slide": { "from": 6, "to": 0, "secs": 0.4 } } }` — higher `ratio`/`index` = more metallic; sliding `index` down gives a struck attack.
- **Warmth / distortion:** `chain` into `drive{amount,shape}` — `tanh` warm, `hard` aggressive, `fold` metallic. Pairs well before a `lowpass`. The shaper is anti-aliased (ADAA) so hard/fold stay clean instead of spraying inharmonic foldback; set `"aa": false` on the node to hear the raw aliasing curve.
- **Struck bodies (bell / glass / metal / coin / UI ping):** the **exciter → resonator** pair — an `impact` into a `modal` bank. `chain[ {type:impact, hardness:0.85}, {type:modal, modes:[{freq,decay,gain}, …]} ]`. Each mode is a damped sine; near-harmonic ratios + a long fundamental = bell, off-harmonic ratios = metal, all-short decays = a glass/UI tick. The hammer's `hardness` sets how far up the bank it reaches; `velocity` its energy. Oscillators can't voice these cleanly — modes can.
- **Fat lead / pad (supersaw):** `{ "type": "super", "wave": "sawtooth", "freq": 220, "voices": 7, "detune_cents": 20 }` — more `voices` / `detune_cents` = wider and thicker. Great through a `lowpass` filter envelope, or as a `mix` layer under a melody.
- **Morphing sweep (wavetable):** `{ "type": "wavetable", "wave": "basic", "freq": 110, "position": { "lfo": { "shape": "sine", "rate": 0.5, "depth": 0.5, "center": 0.5 } } }` — `position` (0..1) crossfades across a built-in table set: `basic` (sine → triangle → square → saw), `harmonics` (a saw growing its partial count — a pure brightness ramp), `formant` (vowel-ish stacks a → e → i → o → u — sweep slowly for vocal morphs), `metallic` (sparse clang stacks). Modulating `position` is the signature move: a slow LFO is an evolving pad, an `env` is a struck brightness decay. Tables are generated at build time (zero assets), band-limited to 32 partials — keep bright positions under ~600 Hz at 44.1 kHz to avoid foldover.
- **Surgical EQ:** `peak{cutoff,q,gain_db}` boosts/cuts a band (e.g. `+6 dB` at 3 kHz for presence); `lowshelf`/`highshelf{cutoff,gain_db}` tilt the lows/highs; `notch{cutoff,q}` removes a resonance or hum. Read `spectral_centroid_hz`, then EQ to hit the brightness you want.

## Apply pro techniques

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
- **Tremolo (streamable amp wobble):** `tremolo{rate,depth}` — gain swings between `1-depth` and 1 at `rate` Hz (defaults 6 / 0.5). Unlike a modulated `gain` (which renders offline but can't stream), tremolo is a closed form of the absolute sample index, so it streams natively and byte-identically to the offline render.
- **Real spaces (`convolve`, offline-only):** `convolve{decay,size,predelay,damp,mix}` is a convolution reverb whose impulse response is *synthesized* — zero assets, no IR files: a noise burst decaying to −60 dB over `decay` seconds (RT60-ish, default 1.5), length-capped by `size` (0 = `decay`), darkened over time by `damp` (0..1, default 0.3), after `predelay` seconds of gap (larger rooms answer later). The IR is deterministic per graph position, so renders are reproducible. Like `reverb`, the tail folds into the document length. A `chain[ impact, convolve ]` is a strike in a hall; small `decay` + high `damp` is a muffled room. **Offline only** — convolution needs the whole input buffer, so streaming refuses it with a named reason (bounce offline, keep the streamed graph causal).
- **Frozen textures (`granular`, offline-only):** `granular{grain_ms,density,pitch,spread,mix}` chops the incoming signal into overlapping Hann grains (`grain_ms` long, `density` per second — defaults 80 / 25), replays each at `pitch` (2 = octave up shimmer, 0.5 = octave-down drone) with deterministic onset jitter and detune of depth `spread`, and crossfades with dry per `mix`. Chain it after any source to smear it into a pad or cloud. **Offline only** — grains read the whole input out of order, so streaming refuses it with a named reason.
- **Glue & loudness:** end a busy chain with `compress{threshold,ratio,attack,release,makeup}`. Watch the
  stats: keep `true_peak_dbfs` below 0, use `loudness_lufs` to match levels across a set, and read
  `crest_factor_db` (big = punchy transient, small = dense/compressed).
- **Variations (round-robin):** `tono_core::vary::mutate(doc, amount, seed)` — a
  Rust API, also `tono vary doc.json -n 8 --amount 0.15 --seed 1` on the CLI —
  with a small `amount` (0.1–0.2) spawns N subtly different takes of a
  footstep / impact / pickup so repeats don't sound identical.
- **Stereo (BGM / ambience):** add a top-level `"stereo"` to the doc —
  `{ "mode": "wide", "amount": 0.6 }` for pseudo-stereo width, or
  `{ "mode": "haas", "ms": 12, "pan": -1 }` for precedence widening. SFX usually stay mono (engine spatialises).

## Edit a doc by path

- Every node has a **path**: `root.inputs[0].freq`, `root.stages[1].cutoff`, `root.stages[1].modes[0].freq`. Paths index into a `chain`'s `stages`, a `mix`/`mul`'s `inputs`, a `seq`'s `notes`, and a `tracks`' `tracks`.
- To change a sound you edit that field in the JSON and re-render — no need to rewrite the whole graph. A field's value can be a number, a modulator object, or a whole node — e.g. set `root.inputs[0].inputs[0].freq` to:
  ```json
  { "slide": { "from": 880, "to": 140, "secs": 0.18, "curve": "exp" } }
  ```
- For programmatic editing there's a small Rust API in `tono_core::edit`:
  `describe(doc)` returns the path → type → params map; `apply_ops(doc, ops)`
  applies a batch of `set{path,value}` · `insert{path,index?,node}` (into a
  `chain`'s `stages` or a `mix`/`mul`'s `inputs`) · `remove{path,index?}` ops in
  one pass; and `morph(a, b, t)` blends two docs.

## Build sounds in layers

Pro SFX are stacks: a transient (the click that says "now"), a body (the identity), a tail (the space). Build them as a **`tracks` root** — the mixer — with one track per component. Track fields:

| field | default | notes |
|-------|---------|-------|
| `id` | backfilled as `layer_<position>` | stable slug, unique within the doc; how edits address the track |
| `node` | — | the track's signal graph |
| `pan` | 0 | −1..1, equal-power |
| `gain` | 1 | 0..2, 1 = unity |
| `at` | 0 | start offset in seconds — a tail 20 ms late is `at: 0.02`, a pre-click 5 ms early against a body at `at: 0.005` |
| `mute` | false | rendered state, not a monitoring convenience: exports ship without muted tracks |
| `automation` | — | song-time `gain`/`pan` lanes |
| `sidechain` | — | duck when another track sounds (below) |
| `bus` / `sends` | — | route to / feed into a mix bus |

A disciplined SFX skeleton is four band-split layers — `sub` / `body` / `top` /
`transient` — each a `chain` of its source into a band-splitting filter and a
one-shot envelope, with a starting `gain`. Fill in the real source per role
(an `fm` or `super` for the body, `noise` → `highpass` for the top, an `impact`
for the transient), then rebalance by reading each layer's contribution.

- **Per-layer stats:** every render's `.stats.json` carries a `layers` array, one entry per track — `{ id, peak_dbfs, rms_dbfs, energy_pct, mute }`, where `energy_pct` is that layer's percentage of the pre-master energy (e.g. `crack 38% • peak −8.1 dBFS | body 52% … | tail 10%`). Nudge a track's `gain` until the split reads right, and edit inside a layer with paths into its `node` (e.g. `root.tracks[0].node.env.d`).
- **One layer per thing you'd fade, pan, time-shift, or analyze separately** — an instrument in a song, a component in an SFX. Use `mix` only for sub-signals that share one envelope/filter; never one track holding a mix of seqs (it makes the per-layer stats useless).
- **Layers are independent by construction:** each track's noise is drawn from a deterministic RNG stream keyed by its `id`, so muting, removing, duplicating, or editing one track never changes a sibling's noise grains. Duplicating a track under a new `id` is a built-in variation — the copy re-grains its noise deterministically from the new id.

**Sidechain ducking between tracks:** a track can carry a `sidechain` that
pulls its level down whenever another track sounds — the classic kick→bass
pump, without nesting a `duck` node:

```json
{ "name": "pump", "duration": 2.0, "version": 2, "root": { "type": "tracks", "tracks": [
    { "id": "kick", "node": { "type": "seq", "bpm": 120, "steps_per_beat": 1, "wave": "sine",
        "env": { "a": 0.001, "d": 0.12, "s": 0.0, "r": 0.02 },
        "notes": [ { "step": 0, "len": 1, "pitch": "A1" }, { "step": 1, "len": 1, "pitch": "A1" } ] } },
    { "id": "pad", "gain": 0.5,
      "sidechain": { "source": "kick", "amount": 0.8, "release": 0.2 },
      "node": { "type": "sawtooth", "freq": 110 } }
] } }
```

- `source` is the driving track's `id`; `amount` (0..1, default 0.8) is the depth, `attack`/`release` (defaults 0.005 / 0.25 s) the ballistics — the same envelope follower the `duck` node uses, so the pump character matches.
- The source renders untouched; only the follower dips, and it ducks when the source actually lands on the bus (the source's `at` offset is honored).
- Several tracks may follow one source, but a source must be a plain track (no follower-of-follower chains — duck directly to the source's source).
- A sidechained mix **streams natively**: the duck envelope advances per sample, so a schema-v2 `tracks` root — sidechains, buses, and all — streams byte-identically to the offline bounce.

## Ship level-matched output

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

## Loop ambience and BGM

For ambience beds, drones, and music that must repeat with no click, set a
top-level `playback`:
```json
"playback": { "mode": "loop", "crossfade_secs": 0.5 }
```

- The renderer extracts the loop region (`start_secs`..`end_secs`, default the whole buffer) and **equal-power crossfades its tail onto its head**, so the rendered file is a seamless loop body (shorter than the source by the crossfade, default 0.1 s).
- The exported WAV carries a `smpl` loop chunk, so host engines loop it at the sample-accurate points with no manual setup.
- Watch the loop seam: if it clicks, raise `crossfade_secs` or match the graph's start/end levels.
- An ambience bed from scratch — slow filter-swept pink noise over a low drone, widened and looped:
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
