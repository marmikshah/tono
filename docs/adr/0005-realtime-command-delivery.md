# ADR 0005: Real-time command delivery

Status: accepted; updated to describe the implemented queue and audio adapter.

## Context

A game loop or Python thread cannot reliably wake on a musical boundary.
Program changes and stingers also need preparation away from the device callback.

## Decision

`Performance` resolves frame/beat/bar/marker/section positions when scheduling
and stores commands in a bounded queue of 4096 entries. Commands execute at their
sample frames during `fill`, with submission order breaking timestamp ties.
A full queue rejects the command and increments the dropped-command metric.

The command queue is owned by Performance; it is not an SPSC command channel.
Python live playback and `Engine::split` use a control-side audio pump. Its SPSC
ring carries interleaved samples to `Renderer`; `Renderer::fill` drains it
without a lock or allocation. An empty ring yields silence. Python's callback
does not acquire the pump/control mutex or GIL.

The shared CPAL `Speaker` wrapper uses a nonblocking source mutex and writes
silence on contention. It grows channel-adaptation scratch on first use or a
larger block. Desktop playback uses this wrapper with a pre-rendered deck,
rather than the Python pump/ring path.

Prepare stinger renders and verify replacement Programs while scheduling.
Swaps crossfade sources, gain changes ramp, and a rejected replacement leaves
the current Program running. Runtime voice pools enforce their configured cap
and priority rules rather than growing without limit.

## Consequences

Headless hosts can drive `Performance::fill` directly. Scratch buffers cover
normal blocks; a larger block can grow them on first use. The allocation proof
covers steady-state fills after the documented warm-up, not arbitrary first-use
blocks. Audio adapters must size and pump their ring for the host's block rate.

Captured command streams replay deterministically. Counting-allocation tests,
command fuzzing, and threaded pump/drain soaks prove software behavior; they do
not establish latency or dropout guarantees on physical audio hardware.

Useful proofs: `crates/tono-core/tests/rt_alloc.rs`, `tests/fuzz_command_stream.rs`,
`tests/soak.rs`, and the tests in `runtime/performance.rs`.
