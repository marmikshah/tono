# Determinism and streaming

A render depends on the graph, seed and sample rate. The current engine uses
fixed f64 transcendental kernels, so the same supported document produces
byte-identical samples across platforms.

## Current formats

SoundDocs and Songs use schema `version: 2` and DSP `engine: 5`. Omitted pins
select these defaults. Explicit unsupported revisions fail validation.
Compiled Programs use format 3 and verify both their content hash and resolved
document on load. Recompile the source song when a bundle is unsupported.

Noise, dust and pluck bursts use deterministic per-node random streams.
Named tracks have independent streams; editing another track does not move
their random draws. Save the source JSON and seed with exported audio.

## Native streaming

`StreamGraph::blockers` explains which parts need a whole-buffer render.
`Player` supplies that render when native streaming is unavailable.

| Streams natively | Uses the buffer-backed Player |
|---|---|
| Source nodes and their modulators, including noise, dust and sequences | SoundFont sampler sequences |
| Every processor, including modulated filter/EQ cutoffs and gain amounts | Nested mixers |
| A tracks root with sidechains, buses and automation | Whole-buffer normalization, loop playback, Haas/Wide stereo treatment |

The bounce and a stream run the same evaluator, so a supported streaming
graph matches the offline render at every block size.
For live compilation, `CompileTarget::Runtime` rejects streaming blockers
instead of accepting a buffer-backed program.
