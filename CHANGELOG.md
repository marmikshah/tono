# Changelog

## 1.0.0 — 2026-06-07

First release. A headless sound studio for AI agents, driven over MCP.

### Instruments & synthesis
- Polyphonic sequencer (`seq`) with a core instrument set: **piano** (detuned
  string pair, velocity brightness, pitch-dependent decay), **e-piano**
  (Rhodes tine), **organ** (tonewheel drawbars + percussion), **strings**
  (ensemble swell), **bass** (filtered + sub), **kit** (full drum kit on the
  General MIDI map), pitched **cowbell**, **pluck** (Karplus-Strong), tunable
  **fm** mallets/bells — plus raw band-limited square/saw/triangle, sine, FM,
  supersaw and three noise colours.
- **`sampler`**: real recorded instruments from any SoundFont (`sf2` path +
  GM program; bank 128 = drum map), rendered deterministically via rustysynth.
- Note-name pitches (`"C4"`, `"midi:60"`), per-parameter modulators
  (`slide`/`lfo`/`arp`/`env`), `swing` + `humanize` groove.

### Production
- **`tracks` mixing console**: per-track equal-power pan and fader onto a true
  stereo bus, master processor chain, decorrelated (Freeverb-spread) reverb
  tails; sampler tracks keep their native stereo.
- Effects: filters + EQ, drive, ringmod, chorus/flanger/phaser, compressor,
  **`duck` sidechain pumping**, bitcrush/downsample, delay, reverb.
- Output stage: LUFS-targeted soft-knee limiting to a true-peak ceiling;
  seamless loops (equal-power crossfade + WAV `smpl` chunk); WAV/FLAC/OGG.

### The agent loop
- Every render returns analysis (peak/true-peak/RMS/crest, ≈LUFS, spectral
  centroid, transients) plus **spectrogram and waveform images**;
  `compare_sounds` reports deltas + similarity.
- Surgical editing by JSON path (`describe_sound` → `set_param` /
  `edit_sound`), 20-deep undo/redo, persistent slug-id library.
- Variations on agent-made sounds: `mutate_sound`, `generate_variants`,
  `humanize`, `morph_sounds`.
- Banks → `sounds.json` manifests + engine files (Godot/Unity/Bevy).

### Sessions
- Every mutating call journaled; `save_session` / `replay_session` (and the
  `sonarium replay` CLI) reproduce a project **byte-for-byte** in a fresh
  directory. Annotated recipe files double as tutorials; nine showcases —
  including the complete *River Flows in You*, its phonk remix, and an
  iconic-sounds pack — replay in CI, with playable renders committed.

### Ops
- stdio + streamable-HTTP transports; self-managing launchd/systemd daemon;
  one-line installer; tagged binary releases (macOS arm64, Linux x86_64,
  Windows); dual-licensed MIT/Apache-2.0.
