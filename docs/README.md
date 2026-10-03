# tono docs

The documentation lives at **https://marmikshah.github.io/tono/** — the sources are this directory.

The site uses VitePress, Vite, and a custom Vue theme. The homepage and listening
room live in `.vitepress/theme/pages/`; the guides retain the documentation layout,
sidebar, and local search. The [architecture guide](https://marmikshah.github.io/tono/architecture.html) and
[decision records](adr/README.md) explain the implementation for contributors.

From the repo root, with Node.js 22.12+ and the pinned Rust toolchain installed:

```sh
npm ci
npm run docs:dev      # development server at /tono/
npm run docs:check    # strict Vue/TypeScript check
npm run docs:build    # static site in docs/.vitepress/dist
npm run docs:preview  # preview the production build
```

The dev, check, and build commands first render 32 SFX previews with the shared
Rust generator: eight starters, each with seeds 42–45. The example
`crates/tono-cli/examples/site_samples.rs` writes mono 48 kHz, 16-bit WAVs,
editable SoundDocs, and waveform metadata into `docs/public/generated/sfx/`.
That directory is ignored by Git and regenerated during the Pages build.
Use `npm run docs:audio` to refresh it after changing a recipe while the dev
server is running.

The Sound Lab plays those rendered previews; generation runs locally through
the CLI or Rust API. The listening room uses the existing tracks in
`public/audio/`. Both pages share one audio player so previews do not overlap.
