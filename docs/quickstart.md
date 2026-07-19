# tono in ten minutes

The zero-assumptions start: from nothing to a sound you built, heard, and
changed on purpose. All you need is a Rust toolchain
([rustup.rs](https://rustup.rs)) — everything else is one install.

## 1. Install the CLI

```sh
cargo install tono
tono --version
```

That's the whole setup. A sound in tono is a **SoundDoc** — a small JSON file
describing a synthesis graph — and the CLI renders it to audio plus pictures
and numbers, so you can author by looking, not guessing.

## 2. Your first sound (2 minutes)

Save this as `blip.json`:

```json
{ "name": "blip", "duration": 0.3, "engine": 4,
  "root": { "type": "mul", "inputs": [
    { "type": "sine", "freq": 880 },
    { "type": "env", "a": 0.002, "d": 0.08, "s": 0.0, "r": 0.05 } ] } }
```

Render it:

```sh
tono render blip.json -o out/
```

You get four files:

- `out/blip.wav` — the audio.
- `out/blip.png` — the **spectrogram** (frequency over time). Look at it first.
- `out/blip_wave.png` — the waveform (loudness over time). Look at it second.
- `out/blip.stats.json` — the numbers: peak, loudness, brightness, attack/decay.

Hear it (needs the playback feature):

```sh
cargo install tono --features play
tono play blip.json
```

## 3. Change one thing, see what changed

Copy `blip.json` to `blip2.json` and, in the copy, change `"freq": 880` to
`"freq": 220`. Now ask what that did:

```sh
tono diff blip.json blip2.json
```

It renders both and compares: the `centroid_hz` row shows the brightness drop
in Hz, and the bottom line gives the sample-domain distance. (Identical docs
answer "sample-identical".) This is the loop: **edit → diff → judge**.

## 4. The loop at full speed

```sh
tono render blip.json -o out/ --watch
```

Leave it running, edit the JSON in your editor, and every save re-renders with
fresh images and stats. Ctrl-C to stop.

## Where next?

- **Make sounds (SFX, UI, impacts)** — the [cookbook](cookbook.md): the full
  node vocabulary and recipes, plus how to judge a sound by its stats.
- **Make music** — the cookbook's `seq` chapter, then the song layer
  ([`song`](https://docs.rs/tono-core/latest/tono_core/song/) on docs.rs):
  patterns, tracks, arrangements.
- **Put it in a game** — [docs/runtime.md](runtime.md): the live Engine/Mixer
  runtime and parametric patches (zero-asset SFX at runtime).
- **Use it from Python** — [crates/tono-py](../crates/tono-py/README.md).
- **No code at all** — the desktop pattern station
  ([build it](../crates/tono-desktop)) or the playground examples
  (`cargo run -p tono-play --example live_band`).

The full map lives in [docs/README.md](README.md).
