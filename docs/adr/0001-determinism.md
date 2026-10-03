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
The desktop replaces invalid or unsupported saved projects with a fresh project
and reports filesystem failures without crashing.

The current renderer uses deterministic f64 transcendental kernels and a
fixed-order FFT in `crates/tono-core/src/det.rs`. RNG streams remain seeded by
node position and named tracks. Offline and streaming evaluators share math;
whole-buffer constructs use a rendered Player fallback.

Golden PCM hashes and block-size sweeps prove the current output. Investigate a
changed hash before accepting it: organisation alone must not change sound.
An intentional sound or format change requires an explicit revision decision
and updated fixtures. Retaining every historical implementation is not policy.

## Consequences

CI checks Linux with pinned and latest Rust. The algorithms avoid platform libm
differences, but Windows/macOS builds are no longer funded or verified.
Historical release claims belong to their changelog entries, not current support.

Useful proofs: `tests/golden.rs`, streaming byte-identity tests,
`tests/equivalence.rs`, and `tests/soak.rs` in `crates/tono-core`.
