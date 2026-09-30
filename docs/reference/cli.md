# The CLI

The `tono` command line — install with `cargo install tono`. Every verb below is the real usage; `tono --help` prints the same text with full flag detail.

| Command | What it does |
|---|---|
| `tono templates` | List the eight game SFX starters. |
| `tono generate TEMPLATE [--seed N] [-n COUNT] [-o DIR]` | Generate four candidates by default, with an audition page, editable JSON, audio, analysis, and a manifest. `--brightness`, `--punch`, and `--variation` accept 0–1; `--sample-rate` defaults to 48000. `--format wav\|flac\|ogg` selects audio encoding. DIR must be empty and defaults to `target/generated/<template>-<seed>`. |
| `tono render FILE.json [-o DIR]` | Render a SoundDoc: the audio (`--format wav\|flac\|ogg`), a spectrogram, a waveform, and a `.stats.json` analysis. `--stems DIR` also writes every track and bus stem (pre-master) as stereo WAVs; `--watch` re-renders on every save. |
| `tono vary FILE.json [-n COUNT]` | Render COUNT deterministic variations of a doc (`--amount 0..1`, `--seed N`) — round-robin takes of a footstep, impact, pickup. |
| `tono schema [sounddoc\|patch]` | Print the JSON Schema of the document format, for editor autocomplete and validation. |
| `tono midi FILE.json [-o FILE.mid]` | Export a SoundDoc's sequences to a Standard MIDI File; `--song` reads a Song instead (each song track becomes a MIDI track, the kit on channel 10). |
| `tono compile SONG.json [-o FILE]` | Compile a Song into a validated, hashed Program bundle — all problems reported in one pass, non-zero exit on failure. `--inspect` prints the machine-readable summary (hash, version pins, roster, estimates, warnings) and writes nothing. |
| `tono import FILE.mid [-o DOC.json]` | Import a Standard MIDI File as a renderable SoundDoc of seq tracks (GM programs map to the built-in voices, channel 10 becomes the drum kit); `--song` imports to a Song instead. |
| `tono diff A.json B.json` | Render both documents and report what changed: loudness, peak, brightness, envelope metrics, and the sample-domain distance. |
| `tono match REF.wav DOC.json` | Score a doc against a reference WAV — how close it is and where it misses (brightness, loudness, envelope, duration). |
| `tono fit REF.wav DOC.json` | Hill-climb the doc's parameters toward the reference WAV — a deterministic seeded search (`--rounds N`, `--amount 0..1`, `--seed N`) that writes the best doc and its final match report. |
| `tono review FILE.json` | Grade a doc against the ship checklist (and `--archetype` targets: laser, coin, jump, impact, ui, footstep, powerup, ambience, bgm); exits non-zero on a FAIL grade. |
| `tono play FILE.json [--secs N]` | Audition a doc through the speakers — feature-gated: `cargo install tono --features play`. |
| `tono presets [NAME] [-o DIR]` | No NAME: list the 16 factory presets. With NAME: render the preset's demo riff through the live Instrument engine. |
| `tono catalog [NAME] [-o DIR]` | No NAME: list the 31 catalog voices by family. With NAME: render the voice's demo (a two-bar groove for the drum kits). |

The render loop — write a doc, render, look at the images, refine — is the [quickstart](/get-started/quickstart); the JSON format itself is [the SoundDoc reference](/reference/sounddoc).

For ready-made game assets, start with [Generate game SFX](/guides/generation).
