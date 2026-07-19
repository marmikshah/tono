# tono docs

The long-form guides, from most-to-least hands-on:

- **[cookbook.md](cookbook.md)** — the `SoundDoc` DSL and how to build sounds:
  the full node vocabulary, recipes (SFX, songs, loops), judging a render by
  its stats, engine revisions. Start here.
- **[runtime.md](runtime.md)** — embedding the live runtime (Engine / Mixer /
  AdaptiveMusic / Instrument) and parametric patches (zero-asset SFX).
- **[examples/](examples/)** — SoundDoc JSON recipes (pinned byte-for-byte in
  the golden-corpus tests) and their rendered audio showcases.
- **[architecture](https://marmikshah.github.io/tono/architecture.html)** —
  how the codebase is put together (lives in `site/` and on the Pages site).

The crate READMEs cover the native faces: [`tono-py`](../crates/tono-py) ·
[`tono-play`](../crates/tono-play) · [`tono-desktop`](../crates/tono-desktop).
