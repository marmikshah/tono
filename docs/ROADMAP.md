# Roadmap to 2.0 — the studio

2.0 is the studio milestone: tono grows from a deterministic engine with a
pattern station into a real music-production app — an FL-style pattern/playlist
workflow with deep routing, fully synthetic, on the same byte-identical core.
The API pass (renames, `#[non_exhaustive]`, the dead-code sweep) shipped in
1.6.0 with the aliases deprecated; 2.0 deleted the old names.

The sequencing rule throughout: the **bounce stays a pure function of
`(project, seed, sample_rate)`** — CI-pinned by the golden corpus — and
playback of an *unedited* project stays byte-identical to the bounce. Live
gestures may deviate until released; that re-scoped contract is what lets the
engine go real-time without giving up its identity.

## Phase 1 — the transport (the one real rewrite)

Everything live today is bounce-then-play: the streaming renderer rejects the
`tracks` root every song compiles to, and edits re-render whole documents.
This phase moves synthesis onto the audio thread behind a typed command queue.

- **Streaming tracks mixer** — per-track block-pull rendering of a `tracks`
  doc, byte-identical to the offline bounce by construction (extend the
  `streaming` module's kernel-sharing pattern; never fork the math).
- **`Transport`** — play/pause/seek/loop-region with a sample-position ↔
  musical-time clock, implementing `AudioSource`.
- **Catalog voices as gated streaming voices** — one `CatalogVoice` seam
  (note_on/note_off/process) shared by the sequencer, the live keyboard, and
  the offline render, ending the split where the best sounds are offline-only.
- **Lock-free command ring** — the audio callback owns the engine; the UI
  sends preallocated typed commands (note events at sample offsets, param
  slots with smoothing, node swaps, transport ops). No mutex on the callback.

*Done when:* a note edited in a playing multi-minute song lands without a
re-render hitch; the catalog piano plays live from a MIDI keyboard; unedited
playback is bit-equal to the bounce (pinned by test).

## Phase 2 — the musical model

Schema work that gets exponentially harder the longer the GUI grows on top of
it — all engine-gated so existing documents replay unchanged.

- **Ticks (PPQ) time base** in `Song`, with a tempo map and time signatures;
  seconds remain a compile-time artifact of `to_doc`.
- **Parameter automation** — `AutoTarget::Param(path)` on the existing
  path-addressing machinery, evaluated at control rate.
- **Buses and sends** in `Node::Tracks` — tracks → buses → master as a
  topologically-sorted DAG; sidechain becomes a bus tap, the per-track reverb
  insert becomes a real send.
- **First-class patterns/clips** — placements editable in place, one pattern
  reused across the playlist without copies.

## Phase 3 — the studio app

- **Frontend rebuild** — TypeScript + Vite + Canvas in `tono-desktop/ui`:
  piano roll, step grid, playlist, mixer. State stays in Rust; the webview
  stays a pure view of it.
- **Live metering** — per-track peak/RMS taps and a streaming LUFS +
  spectrogram for the master bus (the analysis loop, live instead of
  per-bounce).
- **The project format** — a versioned `Project` (song + instrument designs +
  routing + automation + UI state) with the same engine/version pinning
  discipline as `SoundDoc`.
- **MIDI capture** — record a performance into a pattern against the
  transport clock (quantize strength), hot-plugged devices, output-device
  change recovery.
- **Track freeze for free** — a content-addressed render cache keyed on
  `(subgraph, seed, sample_rate, engine)`; determinism makes it exact.

## Cross-cutting, before 2.0 ships

- **Deterministic transcendental kernels (engine 5)** — replace platform libm
  on the render path so byte-identity holds across OS/arch and the golden
  corpus collapses to a single pin table.
- **Typed errors** — `ValidateError` / `EditError` / `SongError` replacing the
  `Result<_, String>` monoculture (`InstrumentError` is the template).
- **Duplication debt** — the `SeqVoice` serde-flatten consolidation and the
  shared biquad coefficient table.
- **CI hardening** — compile gates for the desktop/play crates; the
  `build-test` check made required on `master`.
