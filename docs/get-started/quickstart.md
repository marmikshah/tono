# tono in ten minutes

From nothing to a sound you built, heard, and changed on purpose — five
steps. All you need is a Rust toolchain ([rustup.rs](https://rustup.rs)).

## 1. Install the CLI

```sh
cargo install tono
tono --version
```

## 2. Render your first sound

A sound in tono is a **SoundDoc** — a small JSON file describing a synthesis
graph. Save this as `blip.json`:

```json
{ "name": "blip", "duration": 0.3, "engine": 5,
  "root": { "type": "mul", "inputs": [
    { "type": "sine", "freq": 880 },
    { "type": "env", "a": 0.002, "d": 0.08, "s": 0.0, "r": 0.05 } ] } }
```

```sh
tono render blip.json -o out/
```

`"engine": 5` pins the current DSP kernels: the render is byte-identical on
any machine.

## 3. See it, then hear it

`out/` holds four files:

| File | What it shows |
| --- | --- |
| `blip.wav` | the audio |
| `blip.png` | the spectrogram (frequency over time) — look at it first |
| `blip_wave.png` | the waveform (loudness over time) — look at it second |
| `blip.stats.json` | the numbers: peak, loudness, brightness, attack/decay |

Author by looking, not guessing. To hear it through the speakers, add the
playback feature:

```sh
cargo install tono --features play
tono play blip.json
```

## 4. Change one thing, measure it

Copy `blip.json` to `blip2.json`, change `"freq": 880` to `"freq": 220` in
the copy, and ask what that did:

```sh
tono diff blip.json blip2.json
```

- The `centroid_hz` row shows the brightness drop (881 → 221 Hz here).
- The bottom line is the sample-domain distance; identical docs answer
  "sample-identical".

That is the loop: **edit → diff → judge**. Run it at full speed — every save
re-renders the images and stats (Ctrl-C to stop):

```sh
tono render blip.json -o out/ --watch
```

## 5. Pick your next step

| I want to… | Go to |
| --- | --- |
| Make sounds (SFX, UI, impacts) | [Design sound effects](/guides/sound-effects) — recipes and how to judge a sound by its stats, plus the full [node vocabulary](/reference/sounddoc) |
| Make music | [Compose songs](/guides/songs) — the `seq` chapter, then the song layer: patterns, tracks, arrangements ([docs.rs](https://docs.rs/tono-core/latest/tono_core/song/)) |
| Use it from Python | [the Python guide](/guides/python) — the typed `Song`/`Pattern`/`Program` API; an equivalent song compiles to the same hash from Python or Rust |
| Put it in a game | [Run live & embedded](/guides/live) — the live Engine/Mixer runtime and parametric patches (zero-asset SFX at runtime) |
| Write no code | the desktop pattern station ([build it](https://github.com/marmikshah/tono/tree/master/crates/tono-desktop)) or the playground examples (`cargo run -p tono-play --example live_band`) |
