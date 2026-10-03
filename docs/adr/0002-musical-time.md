# ADR 0002: Exact musical time

Status: accepted; updated to describe the shared compiler/transport conversion.

## Context

Musical positions are fractions: triplets, pickups, and repeated transforms
must retain their positions without accumulating floating-point beat drift.
Rendering and scheduled playback eventually need integer sample frames.

## Decision

Use the normalized rational `Beat` in `crates/tono-core/src/units.rs` for musical
positions and transformations. Convert to the song's step grid explicitly;
off-grid results are errors that identify the note.

Use `Frames`, `SampleRate`, and `Tempo` at the sample-time boundary. At constant
tempo, the rule is `round(beat × 60 / bpm × sample_rate)`, evaluated in f64 with
halves rounded away from zero. Nonpositive positions clamp to frame zero;
tempo conversions floor degenerate BPM at 1.

For tempo maps, walk the segments in f64 seconds and round only the final
sample position. Do not round each segment separately. The compiler,
sequence renderer, and transport use the shared map conversion so note starts,
ends, and scheduled commands land on the same frames.

Meter maps and pickups define the bar-to-beat walk. A bar has no fixed sample
length when tempo or meter changes. A transport derives time from its frame
position and the Program's musical metadata.

## Consequences

Pattern algebra stays exact until a deliberate grid or sample conversion.
Floating-point seconds remain an output representation, not the authoritative
identity of a musical position. The frame boundary has a specified rounding
rule rather than a host-language default.

Useful proofs: unit tests in `units.rs` and `runtime/transport.rs`,
`crates/tono-core/tests/fuzz_time.rs`, and the tempo/meter compiler tests.
