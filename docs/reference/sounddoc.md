# The SoundDoc nodes

Every field and node in a **SoundDoc** — the JSON synthesis graph tono renders. `tono schema` prints the machine-readable JSON Schema (for editor autocomplete and validation).

A doc skeleton: `{ "name": ..., "duration": secs, "sample_rate": 44100, "seed": 0, "root": <node> }`. `root` is a single node; every node is a mono signal. Multiply a source by an envelope (`mul`), layer sources with `mix`, pipe a source through processors with `chain`. Any numeric param is a constant, a note name, or a modulator.

## Node vocabulary

Document-level fields:

| field | default | notes |
|-------|---------|-------|
| `name` | — | label; names the output files |
| `duration` | 0.3 | length in seconds |
| `sample_rate` | 44100 | Hz |
| `seed` | 0 | drives every `noise` source, `dust` train, and pluck burst |
| `version` | 2 | current document schema; other revisions are rejected |
| `engine` | 5 | current deterministic DSP kernels; other revisions are rejected |
| `stereo` | mono | `wide` / `haas` treatment on the final render |
| `normalize` | unset ⇒ a transparent −0.1 dBFS sample-peak safety limit only | loudness match + true-peak limit |
| `playback` | `oneshot` | or `loop` |
| `root` | — | the signal graph |

**Sources** (output audio):

| node | fields | what it is |
|------|--------|------------|
| `sine` / `triangle` / `sawtooth` | `freq` | basic waves |
| `square` | `freq`, `duty` = 0.5 | pulse wave; modulate `duty` for PWM |
| `noise` | `color` = `white` | `pink` −3 dB/oct (wind/rumble), `brown` −6 dB/oct (booms) |
| `fm` | `freq`, `ratio`, `index` | 2-op FM — bells, e-piano; slide `index` down for a struck attack |
| `super` | `wave` = `sawtooth`, `freq`, `voices` = 7 (1..=16), `detune_cents` = 15 | detuned unison stack (supersaw) |
| `wavetable` | `wave` = `basic`, `freq`, `position` = 0 | morphs across built-in tables (`basic`/`harmonics`/`formant`/`metallic`) |
| `seq` | `bpm`, `wave`, `env`, `notes`, … | note sequencer — see [Compose songs](/guides/songs) |
| `impact` | `hardness` = 0.5, `velocity` = 1 | strike exciter; feed a `modal` bank |
| `dust` | `density`, `decay` = 0.02 | Poisson click train; `decay` 0 = bare impulses |
| `env` | `a`, `d`, `s`, `r`, `punch` | ADSR control signal 0..1 (not audio) |

**Combinators:**

| node | fields | what it is |
|------|--------|------------|
| `mix` | `inputs` | sum (layer) all inputs |
| `mul` | `inputs` | multiply (typically source × envelope) |
| `chain` | `stages` | serial pipe: stage 0 is a source, later stages process it |
| `tracks` | `tracks`, `master`, `buses` | stereo mixing console — document root only |

**Processors** (chain stages):

| node | fields | what it is |
|------|--------|------------|
| `gain` | `amount` | scale the signal (modulatable) |
| `lowpass` / `highpass` / `bandpass` / `notch` | `cutoff`, `q` = 0.707 | resonant filters; `notch` removes a narrow band (hum) |
| `peak` | `cutoff`, `q` = 0.707, `gain_db` = 0 | boost/cut one band (surgical EQ) |
| `lowshelf` / `highshelf` | `cutoff`, `gain_db` = 0 | tilt everything below/above the corner |
| `bitcrush` | `bits` (1..16) | amplitude quantize — crunch |
| `downsample` | `factor` | sample-rate reduction — lo-fi grit |
| `delay` | `secs`, `feedback` = 0 | feedback echo / comb |
| `reverb` | `room` = 0, `mix` = 0 | Schroeder-style reverb |
| `modal` | `modes` = `[{freq, decay=0.4, gain=1}]` (1..=64), `mix` = 1 | resonator bank — a struck body |
| `drive` | `amount`, `shape` = `tanh`, `aa`? | waveshaper: `tanh` warm, `hard` clip, `fold` metallic; ADAA by default; `aa: false` selects the raw curve |
| `ringmod` | `freq` | ring modulation — metallic, robotic |
| `tremolo` | `rate` = 6, `depth` = 0.5 | amp wobble; streams natively |
| `chorus` | `rate` = 1.5, `depth` = 0.5, `mix` = 0.5 | thickening / width |
| `flanger` | `rate` = 0.25, `depth` = 0.5, `feedback` = 0.5, `mix` = 0.5 | jet sweep / metallic whoosh |
| `phaser` | `rate` = 0.4, `depth` = 0.5, `feedback` = 0.3, `mix` = 0.5 | swept all-pass notches |
| `duck` | `trigger`, `amount` = 0.8, `attack` = 0.005, `release` = 0.25 | sidechain pump; the trigger renders silently |
| `compress` | `threshold`, `ratio`, `attack` = 0.005, `release` = 0.08, `makeup` = 0 | glue / loudness |
| `convolve` | `decay` = 1.5, `size` = 0, `predelay` = 0, `damp` = 0.3, `mix` = 0.35 | synthesized-IR convolution reverb — **offline only** |
| `granular` | `grain_ms` = 80 (5..=500), `density` = 25, `pitch` = 1, `spread` = 0.3, `mix` = 0.5 | grain cloud / frozen pad — **offline only** |

**Modulators** (any numeric param, in place of a constant):

| modulator | fields | what it does |
|-----------|--------|--------------|
| constant / note name | `440`, `"C4"`, `"F#3"`, `"Gb5"`, `"midi:60"` | note names resolve to Hz (A4 = 440) |
| `slide` | `from`, `to`, `secs`, `curve` = `lin`\|`exp` | glide, then hold at `to`; `exp` reads as natural pitch glide |
| `lfo` | `shape` = `sine` (`square`/`triangle`/`saw`), `rate`, `depth`, `center` | periodic oscillation around `center` |
| `arp` | `steps`, `rate` | cycle values at `rate` steps/sec |
| `env` | `a`, `d`, `s`, `r`, `punch` + `from`, `to` | ADSR mapped onto the `from`→`to` range |
| `rand` | `from`, `to`, `rate`, `seed` = 0 | smooth random walk; self-seeded and edit-stable |
