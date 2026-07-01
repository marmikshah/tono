<p align="center">
  <img src="docs/logo.png" width="112" alt="tono — a pluck waveform on a dark tile">
</p>
<p align="center">
  <img src="docs/logo-wordmark.png" width="384" alt="tono">
</p>

<p align="center"><strong>A deterministic sound engine — author a synthesis graph and get byte-identical audio, from Rust, a command line, or a live keyboard.</strong></p>

<p align="center">
  <a href="https://github.com/marmikshah/tono/actions/workflows/ci.yml"><img src="https://github.com/marmikshah/tono/actions/workflows/ci.yml/badge.svg" alt="CI"></a>
  <a href="https://crates.io/crates/tono-core"><img src="https://img.shields.io/crates/v/tono-core" alt="crates.io"></a>
  <img src="https://img.shields.io/badge/license-MIT-8c6ee6" alt="license">
</p>

<p align="center">
  <img src="docs/river-flows-spectrogram.png" width="640" alt="spectrogram of River Flows in You, 800 notes on the sampled piano">
</p>
<p align="center">
  <img src="docs/river-flows-waveform.png" width="320" alt="waveform of the piano piece">
  <img src="docs/band-demo-waveform.png" width="320" alt="waveform of the band demo groove">
</p>

<p align="center"><em>Everything you can hear in <a href="docs/examples/audio/">docs/examples/audio</a> — a complete piano piece, a four-instrument band, game-ready BGM loops, and an iconic-sounds pack — was rendered by this engine. The logo and wordmark were drawn with <a href="https://github.com/marmikshah/atelier">atelier</a>, tono's pixel-art sibling.</em></p>

## What it is

tono is a **deterministic synthesis engine**: a sound is a symbolic graph
(oscillators → envelopes → filters → modulation → mix), and rendering it is a
**pure function of `(graph, seed, sample_rate)` → byte-identical audio**. The
same document sounds identical whether you render it offline, stream it in real
time, or play it as an instrument. No API keys, no network — just DSP.

- **A real studio, headless** — a polyphonic sequencer with a core instrument
  set (piano, e-piano, organ, strings, bass, a GM drum kit, plucked string, FM
  mallets) plus raw band-limited oscillators, FM, supersaw and three noise
  colours for synthesis and SFX.
- **Playable instruments** — turn any patch into a pitched, polyphonic
  **instrument** with velocity, pitch bend, glide, mono/legato, unison, a
  filter-brightness knob, and vibrato/tremolo/wobble modulation. Ships an
  11-preset factory bank and a playable **drum kit**.
- **Songs & adaptive music** — arrange instruments and patterns into a **song**
  on a timeline, or drive **adaptive game music**: intensity-layered stems that
  cross-fade with the action, plus one-shot stingers.
- **A mixing console** — per-track pan/gain onto a true stereo bus, a master
  processor chain, decorrelated reverb tails, sidechain ducking, swing/humanize.
- **Real recorded instruments** — the `sampler` voice plays any SoundFont
  (point it at a free GM bank) — sampled grands, basses, ensembles, GM drums —
  and stays deterministic.
- **An ear for critique** — peak/true-peak/RMS/crest, ≈LUFS, spectral centroid,
  attack/decay/onset descriptors, and a **spectrogram + waveform** image, so
  "does it sound right?" becomes numbers and pictures you can act on.
- **Game-ready output** — WAV/FLAC/OGG, and the pure core compiles straight into
  a game for real-time, per-instance SFX.

## Use it from the command line

`tono render` is the whole loop: write a `SoundDoc` (JSON), render it, and look
at what came out.

```sh
tono render docs/examples/blip.json -o out/
#   out/blip.wav          the audio            (--format wav|flac|ogg)
#   out/blip.png          the spectrogram      ← look at this
#   out/blip_wave.png     the waveform         ← and this
#   out/blip.stats.json   peak / RMS / LUFS / spectral / transient analysis
```

That's all an agent needs to author sound by inspection: emit a doc, render,
read the two images + the stats, refine the doc, repeat.

## Use it from code

`tono-core` is the published engine crate — a pure, deterministic library.

```rust
use tono_core::dsl::SoundDoc;
use tono_core::render;

let doc: SoundDoc = serde_json::from_str(r#"{
    "name": "blip", "duration": 0.3, "engine": 2,
    "root": { "type": "mul", "inputs": [
        { "type": "sine", "freq": 880 },
        { "type": "env", "a": 0.002, "d": 0.08, "s": 0.0, "r": 0.05 } ] }
}"#)?;

let samples: Vec<f32> = render::render(&doc); // byte-identical every run
```

Play a factory instrument, compose a song, hit a drum kit, or run adaptive
music — all deterministic, all one `AudioSource`:

```rust
use tono_core::instrument::{Instrument, Note};
use tono_core::{presets, song::{Song, note}, drumkit::DrumKit, adaptive::AdaptiveMusic};
# fn demo() -> anyhow::Result<()> {
let mut lead = Instrument::new(presets::preset("vibrato_lead").unwrap(), 48_000)?;
lead.note_on(Note::C4, 0.9);                      // play it live

let mut song = Song::new("groove", 120.0);
song.add_track("drums", tono_core::dsl::SeqWave::Kit, /* env */ Default::default());
song.add_pattern("beat", 1, vec![note(0, 2, "midi:36"), note(4, 2, "midi:38")]);
song.arrange_repeat("drums", "beat", 0, 4);
let doc = song.to_doc().map_err(anyhow::Error::msg)?; // → a normal SoundDoc

let mut kit = DrumKit::general_midi(48_000);
kit.note_on(Note(36), 1.0);                       // kick

let mut music = AdaptiveMusic::new(48_000);       // reactive game music
music.set_intensity(0.9);
# Ok(()) }
```

The programmatic playground hears any of these in a couple of lines
(`cargo run -p tono-play --example song` / `drums` / `presets` / `adaptive`).

## Install

```sh
cargo add tono-core      # the engine, as a library (games, tools)
cargo install tono       # the `tono render` CLI
```

Sampled instruments need a free General MIDI SoundFont once
(FluidR3 GM, GeneralUser GS): `wave: "sampler", sf2: "/path/to/gm.sf2",
sf2_preset: 0` (0 grand, 32 bass, 48 strings; `sf2_bank: 128` = GM drums).

## Design by hand, play it, ship it into a game

The same deterministic engine drives more faces — one `SoundDoc`, rendered
byte-identically by all of them:

- **A real-time runtime** — a byte-identical **streaming renderer** and an
  embeddable `Engine`/`Mixer`, so a patch authored offline plays live,
  block-by-block, driven by gameplay. Ship a parametric
  [patch](docs/runtime.md) and render endless per-instance SFX variations with
  **zero baked files** — the pure core compiles into your game.
- **A native desktop studio** — a Tauri app with real-time audio: play your
  patch like an instrument from the computer keyboard or a MIDI controller
  (`make desktop`). Optional — never part of the default build or CI.
- **A programmatic playground** — build a sound or instrument in a few lines of
  Rust and hear it (`make play`).

## Iconic sounds

Recognizable classics rebuilt from scratch, with playable renders in
[docs/examples/audio/](docs/examples/audio/) — the ▶ links play right on GitHub:

| Sound | Play | The trick |
|---|---|---|
| retro-coin | [▶](docs/examples/audio/retro-coin.mp4) | B5 grace note into a held E6 — the interval *is* the sound |
| jump-8bit | [▶](docs/examples/audio/jump-8bit.mp4) | exponential square sweep, gone at sustain 0 |
| waka | [▶](docs/examples/audio/waka.mp4) | per-note pitch slides alternating up/down — the chomp drawn into the notes |
| nokia-tune | [▶](docs/examples/audio/nokia-tune.mp4) | 13 notes of Gran Vals on the Karplus-Strong pluck |
| deep-note | [▶](docs/examples/audio/deep-note.mp4) | supersaw tracks gliding from a scattered cluster onto a five-octave D chord |

## Determinism

Rendering is a pure function of the document, so audio is a stable target you
can **test, diff, and cache**. The real-time streaming path is byte-identical to
an offline bounce (verified by a fuzzer in CI), and byte-changing kernel
upgrades are gated behind a document `engine` revision so old sounds never
change. Render the same `SoundDoc` twice and you get the same bytes.

## More

- [docs/cookbook.md](docs/cookbook.md) — the `SoundDoc` DSL, the node
  vocabulary, the instrument table, and worked recipes (validated in CI).
- [docs/runtime.md](docs/runtime.md) — embedding the engine + parametric patches.
- [docs/examples/midi_to_seq.py](docs/examples/midi_to_seq.py) — convert a MIDI
  file to a `seq`.
- `make help` lists every target — `make verify` mirrors CI (fmt + clippy +
  test); `make desktop` / `make play` build the native faces.

## License

[MIT](LICENSE) — permissive, no warranty.
