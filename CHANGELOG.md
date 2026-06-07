# Changelog

## 0.1.0 — unreleased

Initial public release.

- **Synthesis-graph DSL** (`SoundDoc`): band-limited square/saw/triangle, sine,
  FM, supersaw, three noise colours, a polyphonic `seq` note-sequencer with
  note-name pitches (`"C4"`, `"midi:60"`); ADSR envelopes; `mix`/`mul`/`chain`
  combinators; filters + EQ, drive, ringmod, chorus/flanger/phaser, compressor,
  bitcrush/downsample, delay, reverb; `slide`/`lfo`/`arp`/`env` modulators on
  any numeric parameter. Semantic validation with agent-actionable errors.
- **Deterministic rendering**: a sound is a pure function of
  `(graph, seed, sample_rate)` — same input, identical bytes. Seamless loop
  bodies (equal-power crossfade + WAV `smpl` chunk), Haas/wide stereo, and an
  opt-in LUFS-targeted, true-peak-limited output stage.
- **Analysis feedback**: peak/true-peak/RMS/crest, ≈LUFS, spectral centroid,
  attack/decay/onset/silence descriptors, plus spectrogram and waveform PNGs.
- **MCP tool surface** (25 tools): author/refine, path-addressed surgical
  editing with describe/undo/redo, variation tools (mutate, variants, humanize,
  morph), compare, loops, WAV/FLAC/OGG export, banks with `sounds.json`
  manifests and engine files (Godot/Unity/Bevy).
- **Reproducible sessions**: every mutating tool call is journaled;
  `save_session` / `replay_session` reproduce a whole project byte-for-byte.
- **Transports & ops**: stdio, streamable HTTP, and a self-managing background
  daemon (launchd / systemd --user).
