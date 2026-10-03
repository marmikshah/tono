# Performance checks

Criterion benchmarks live in `crates/tono-core/benches/render.rs`. Run them on
Linux with the pinned Rust toolchain from the workspace root:

```sh
cargo bench --locked -p tono-core --bench render
```

For a quick execution check, append `-- --test`. Compare timings on the same
hardware, compiler, feature set, and inputs. CI runs functional and allocation
proofs; it does not enforce wall-clock thresholds on shared runners.

## Historical measurements

These measurements were recorded for v1.10.0-rc.1 on Apple Silicon/macOS in a
release build. They are retained as context, not current Linux budgets or
claims that macOS builds are still tested.

| Bench | What it measures | Historical reference |
|---|---|---|
| `render/blip_osc_env` | sine + ADSR micro-render (0.3 s) | ~0.31 ms |
| `render/tracks_mix_automation_master_reverb` | mixer, lanes, and master chain | ~3.5 ms |
| `render/seq_piano` | additive piano voice (1 s) | ~8.8 ms |
| `render/fx_chain_reverb_delay_compress` | wet FX chain | ~1.9 ms |
| `streaming/streamgraph_fill_512` | 512-frame streaming block | ~0.89 ms |
| `compile/song_to_program` | representative multi-track compile | ~0.21 ms |
| `scheduling/performance_fill_512` | 512-frame Performance block | ~19 µs |
| `mixing/tracks_stems_8` | eight-track stems with a bus | ~72 ms |

## Interpret the results

A 512-frame block at 48 kHz lasts 10.67 ms. Measure processing cost against the
chosen device block interval and ring depth; offline authoring latency and
real-time scheduling measure different work. Listen on actual hardware before
claiming dropout-free playback.

`tests/rt_alloc.rs` counts allocations after the documented scratch warm-up.
The ignored `tests/soak.rs` checks deterministic scheduled playback and threaded
pump/drain behavior. Command queue capacity is 4096; overflow rejects and counts
commands. These proofs do not bound OS scheduling delays or all memory use.

Program estimates describe frames, note events, and concurrent note intervals
(the sum of each track's maximum). `memory_bytes` is the stereo f32 output
estimate, `frames × 8`; it is not total peak RAM for track bounces, effects,
SoundFonts, or adapter buffers. `tests/estimates.rs` checks those estimates
against representative renders, including frame rounding boundaries.
