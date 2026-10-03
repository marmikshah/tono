# tono-web

A lean WebAssembly adapter for `tono-core`. The browser synthesizes ordinary
SoundDoc JSON with the same Rust DSP used by the CLI. It needs no shipped audio
files, JavaScript DSP implementation, wasm-bindgen, or native host imports.

From the repository root:

```sh
rustup target add wasm32-unknown-unknown
node scripts/build-web.mjs
cargo test --locked -p tono-web
# After generating the site recipes with npm run docs:audio:
node --experimental-strip-types scripts/web-engine.test.mjs
```

The build copies the module to `docs/public/generated/engine/tono.wasm`. The site
loads it lazily in a Web Worker, copies stereo PCM into Web Audio, and creates
WAV downloads locally. The visual studio submits edited SoundDocs directly to
the same worker; exact serialized content keys keep changed graphs out of stale
PCM caches. Native tests and the Node integration check cover error
recovery and exact native/WASM sample bits, including stereo mixers, looping,
normalization, instruments, convolution, and granular processing.

## ABI

Instantiate the module without imports and read its exported `memory`.

1. Require `tono_abi_version() === 1` and the supported `tono_engine_version()`.
2. Call `tono_prepare(byteLength)` and copy UTF-8 JSON to the returned pointer.
3. Call `tono_render()`. One means success; zero means failure.
4. On success, copy `tono_frames()` f32 samples from `tono_left_ptr()` and
   `tono_right_ptr()`. Both channels use `tono_sample_rate()`.
5. On failure, copy `tono_error_len()` UTF-8 bytes from `tono_error_ptr()`.
6. Call `tono_reset()` after copying either result.

A zero prepare pointer means failure. Read the error, then reset. Preparing,
rendering, or resetting invalidates previous result views. Rendering can grow
WASM memory, so read `memory.buffer` again after render. The module owns one
request at a time; a worker must serialize access. Reset releases owned buffers;
WebAssembly linear memory can retain its allocated capacity for reuse.

The browser adapter disables native SoundFont loading and image analysis. It
supports synthesized graphs up to 30 seconds at 8–48 kHz, 1 MiB of JSON, 128
nodes, and 32 levels. It also checks conservative graph, note-work and convolution
workspace budgets before DSP allocations. Graph intermediates retain the original
32-million frame-work cap. Musical scores use a separate 64-million note-work
cap, counting actual note gates, scratch buffers, and instrument setup instead
of charging every event for the whole song. The authored BGM catalog is checked
against both caps. Full native Tono supports larger documents and SoundFont
samples. Live note playback could add a streaming/AudioWorklet adapter to the
existing core instrument runtime.
