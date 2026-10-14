# ADR 0001: Determinism and current formats

Status: accepted; updated for the current-format cleanup.

## Context

Sounds are committed documents, rendered through Rust, the CLI, native playback,
or Python. A refactor must preserve their samples and the agreement between an
offline bounce and streaming playback. Historical DSP implementations added
separate behavior paths without serving the current product.

## Decision

Support SoundDoc/Song schema 2 and DSP engine 5 only. Omitted document pins
select those current defaults; explicit unsupported revisions fail validation.

The current renderer uses deterministic f64 transcendental kernels in
`crates/tono-core/src/det.rs`. RNG streams remain seeded by node position and
named tracks. One block evaluator serves both the bounce and streaming;
whole-buffer constructs use a rendered Player fallback.

Golden PCM hashes and block-size sweeps prove the current output. Investigate a
changed hash before accepting it: organisation alone must not change sound.
An intentional sound or format change requires an explicit revision decision
and updated fixtures. Retaining every historical implementation is not policy.

Removing a node type is not a revision: a document that never used it renders
the same bytes under the same `engine` pin, and one that did fails to parse
with the unknown type named. 1.11.0 removed `wavetable`, `bitcrush`,
`downsample`, `ringmod`, `flanger`, `phaser`, `duck`, `convolve` and
`granular` on that basis; schema 2 and engine 5 stay.

## Consequences

CI checks Linux with pinned and latest Rust. The algorithms avoid platform libm
differences, but Windows/macOS builds are no longer funded or verified.
Historical release claims belong to their changelog entries, not current support.

Useful proofs: `tests/golden.rs`, streaming byte-identity tests,
`tests/equivalence.rs`, and `tests/soak.rs` in `crates/tono-core`.
