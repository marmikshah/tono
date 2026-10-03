# ADR 0003: Song compilation and the Program artifact

Status: accepted; updated for Program format 3.

## Context

A host needs a validated render graph and musical metadata without re-running
the song compiler on every load. Duplicating document facts in the bundle can
make transport, inspection, and playback disagree after a change or edit.

## Decision

`Song::compile()` collects diagnostics and builds a Program containing the
resolved SoundDoc, compile target, musical metadata, resource estimates, and
canonical integrity hash. Program format 3 is the only supported bundle format.

The document owns its name, schema/engine pins, sample rate, and duration.
Metadata owns musical facts: tempo/meter maps, pickup, sections, markers, length
in bars, and the track roster. Warnings and streaming capabilities are derived
from the current document rather than serialized as duplicate state.

Track names are persistent identities. Roster order is declaration order;
inspection indices identify positions in that roster. Runtime resource and
instance handles belong to their owning runtime, not a historical global ID
slot table.

The bundle hash is FNV-1a over canonical sorted-key JSON of every serialized
field except `hash`. It detects accidental corruption; it is not a security
signature. Load checks the exact revision, hash, document validation, and
runtime target constraints. Scheduled swaps use the same verification and
retain the last valid source if verification fails.

## Consequences

Rendering and transport consult one authoritative document. Rust and Python
compile equivalent songs to identical artifacts. Invalid bundles return
structured errors; loaders do not silently migrate or recompile them.

Useful proofs: `tests/fuzz_bundle.rs`, `tests/equivalence.rs`, compiler tests,
and `runtime/performance.rs` in `crates/tono-core`.
