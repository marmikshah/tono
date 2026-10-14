# tono docs

The documentation site at **https://marmikshah.github.io/tono/** is built
from this directory with VitePress. Guides and references are Markdown; the
sound browser, Sound Studio and lessons are Vue pages under
`.vitepress/theme/pages/` that render audio in the browser with the
`tono-web` WebAssembly build of the same core DSP.

From the repo root, with Node.js 22.12+ and the pinned Rust toolchain:

```sh
npm ci
rustup target add wasm32-unknown-unknown
npm run docs:dev      # development server at /tono/
npm run docs:check    # export the site recipes, then strict Vue/TypeScript check
npm run docs:test     # transport, editor, WAV and WASM/native equivalence checks
npm run docs:build    # static site in docs/.vitepress/dist
npm run docs:preview  # preview the production build (binds 127.0.0.1:4173)
```

`docs:audio` exports the starter, library and background-music recipes from
the `tono-content` crate into `docs/public/generated/sfx/` (git-ignored,
rebuilt by the Pages workflow); run it after changing a recipe. `docs:engine`
rebuilds `public/generated/engine/tono.wasm`; run it after changing DSP code
while the dev server is running. The browser adapter's limits and C ABI are
documented in `crates/tono-web/README.md`.

Code examples in `get-started/` and `guides/` use `::: code-group` blocks
labelled `[Rust]` and `[Python]`; keep them complete and run them against the
checkout when an API changes.
