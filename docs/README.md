# tono docs

The documentation lives at **https://marmikshah.github.io/tono/** — the sources are this directory.

The site uses VitePress, Vite, and a custom Vue theme. The homepage and listening
room live in `.vitepress/theme/pages/`; the guides retain the documentation layout,
sidebar, and local search. The [architecture guide](https://marmikshah.github.io/tono/architecture.html) and
[decision records](adr/README.md) explain the implementation for contributors.

From the repo root, with Node.js 22.12+ and the pinned Rust toolchain installed:

```sh
npm ci
rustup target add wasm32-unknown-unknown
npm run docs:dev      # development server at /tono/
npm run docs:check    # strict Vue/TypeScript check
npm run docs:build    # static site in docs/.vitepress/dist
npm run docs:test     # WAV encoding and WASM/native equivalence + catalog checks
npm run docs:preview  # preview the production build
```

`docs:audio` exports editable SoundDocs and measured waveform metadata for
the 32 starter previews, the sound library's 64 recipes × six variations
(384 sounds), and 12 background music themes × three arrangements (36 loops).
BGM exports include editable Song scores as well as their compiled SoundDocs.
The exporter writes no WAV/OGG files. These generated files
live in `docs/public/generated/sfx/`, which is ignored by Git and rebuilt
during the Pages build. Run `npm run docs:audio` after changing a recipe.

The dev and build commands also run `docs:engine`: a lean `tono-web` build
for `wasm32-unknown-unknown`, copied to `public/generated/engine/tono.wasm`.
It uses the existing core DSP with analysis and native SoundFont loading
disabled. There is no separate JavaScript synthesizer or wasm-bindgen CLI.
Run `npm run docs:engine` after changing DSP code while the server is running.

The Sound Lab, `/sounds`, `/bgm` and `/create` lazily load this engine in a worker. Each play
request loads a recipe, validates it, and renders stereo PCM away from the
UI thread. Web Audio plays the result; WAV downloads encode those samples
locally, including loop metadata. A 24 MiB in-memory LRU cache avoids repeated
rendering; the worker and player release their resources on page navigation.
Musical loop exports preserve their bar duration, with warm-up and crossfade
material outside the playback region so the rhythm stays on the grid.
The listening room continues to use the tracks in `public/audio/`. One
transport per page keeps music and synthesized previews from overlapping.

The browser adapter limits documents to 1 MiB, 30 seconds, and 48 kHz, plus
graph complexity/work bounds. Sequence budgets count actual note gates and
scratch, with separate limits for graph intermediates and musical voice work.
Errors appear on the page and failed or stale
play requests never replace a more recent selection. `docs:test` checks WAV
encoding, exact native/WASM sample equivalence across DSP fixtures, and every
exported recipe in the actual WASM module. Pages CI runs these checks.

The Sound Studio at `/create` builds SoundDocs from draggable instrument layers
and a 16-step note grid. Its controls cover tempo, duration, seed, note length,
transposition, volume, pan, envelopes, filtering and delay. Users can undo/redo,
save and reopen their own projects, and export a recipe or WAV. Drafts stay in
the browser's local storage. The editor uses the same worker's direct-document
API; it creates no server-side project or audio file. A streaming AudioWorklet
adapter for live notes remains a later extension of the current bounce player.
