# tono 2.0: trim and harden

Status: proposal, 2026-10-13. A breaking major release is acceptable.

## Goal

Make tono a small, trustworthy audio engine that other projects can depend
on: one evaluator, a frozen public API with semver guarantees, a lean
dependency tree, fast cross-platform CI, and a repository where everything
that is checked in is used, tested and documented.

What "professional grade" means here, concretely:

- `tono-core` has one DSP implementation, no content packs, no dead `pub`
  items, and builds with 30 or fewer crates in its lean configuration.
- Every public item is documented, every DSP node is golden-pinned and
  streaming-verified, and API breakage is detected by CI, not by users.
- CI proves Linux, macOS and Windows builds (public repos get those runners
  for free; the old "not funded" rationale no longer applies).
- The docs site documents the engine. Demo applications live elsewhere.

## Where the repo stands (master, 2026-10-13)

| Area | Size | Finding |
|---|---|---|
| `tono-core` | 39k lines (≈17k code, ≈11k tests, ≈8k authoring/content, rest fixtures) | Two complete DSP evaluators (offline `render/`, realtime `streaming/`) kept byte-identical by tests rather than by construction; ≈900 lines of per-sample arithmetic written twice, duck follower written four times. 18 of 39 node types are used by no shipped content. `library.rs` (1.7k) and `bgm.rs` (0.9k) exist only to feed the docs site build. `edit::describe/morph`, `vary::humanize`, `music::{Voicing, Inversion}`, most of `units.rs` have no callers. |
| `tono-cli` | 4.4k lines, 90 deps | Hand-rolled argument parsing; 15 subcommands, four (`diff`, `match`, `fit`, `review`) undocumented; `flacenc` + `vorbis_rs` (a C build) always on; a 19-line `lib.rs` that exists only for one test and one example. |
| `tono-desktop` | 1.6k lines, **270 of the 528 lockfile packages** | Tauri skeleton with three fixed instruments; `bundle.active = false`, never shipped, superseded by the web Sound Studio; compiled twice per PR and the sole reason CI installs WebKitGTK. |
| `tono-py` | 3k Rust + 1.4k example lines | Well-structured wrappers; stubs in sync but unchecked; `song.rs` (1.6k) hand-mirrors the catalog and processor list; seven full compositions in `examples/` are unreferenced and untested. |
| `tono-web`, `tono-play` | 0.7k / 1k | Fine. `tono-play` is unpublished so the CLI re-implements its player. |
| Docs site | 4.4k lines Vue/TS | ≈2.5k lines are two applications (Sound Studio, library browser) rather than documentation; 526 lines of dead components; 9.3 MB of mp4s serve one Showcase page. |
| CI | ≈20 compute-minutes per PR | Pinned and stable gates run identically; a `--release --workspace` build duplicates the test build; `docs:audio` runs twice; Pages runs on every PR. |
| Tests | ≈600, green | Golden/equivalence/rt-alloc/time-fuzz/soak suites are excellent. Streaming identity and validation fuzz are hand-expanded per node (≈1.4k lines that a table would replace). |

Baseline: `cargo test --locked` passes in ≈1m40s locally.

## Principles for the work

1. Every phase leaves CI green and the golden corpus byte-identical, except
   where the phase explicitly introduces engine revision 6.
2. Delete before refactoring. Removing dead code and unshipped apps first
   shrinks the surface every later change has to preserve.
3. Move, don't lose. Content packs and demo apps go to sibling repositories
   that depend on the published crates, not into `/dev/null`.
4. One PR per bullet below; no omnibus PRs.

## Phase 0: hygiene (no API change, ~1 week)

- Delete dead code: `edit::{describe, morph}` and tests; `vary::humanize` and
  `transpose_*`; `music::{Voicing, Inversion}`, unused `Scale`/`ChordQuality`
  helpers and hand-written serde impls; `units::{Frames, SampleRate, Tempo,
  beat_to_frames, meter_at}`; `render::render_graph`; the unused `Serialize`
  derives on `Recipe`/`Composition`; dead Vue (`CodeExample`, `MusicCard`,
  `SoundCard`, `SoundPreview`, `CoverArt`) and the starter block in
  `data/catalog.ts` + `site_samples.rs:33-56`.
- Dedupe helpers: `doc_frames`, `layer_id_or_default`, `one_pole`,
  `stamp_sample_rate`, one canonical `render_stereo` (make `Program`,
  `LoopBuffer`, `Performance::stinger` call it).
- Fix stale docs: `runtime/mod.rs` header, `edit.rs` reference to
  `refine_sound`, `lib.rs` crate map (omits `bgm`), install page using
  `git clone` while crates are on crates.io.
- CI: drop the `stable` gate from PRs (keep weekly), drop the "Build all
  release targets" step, path-filter Pages on PRs, run `docs:audio` once.
- Add `[package.metadata.docs.rs] all-features = true` to published crates.

Expected: −1.5k lines, −8 CI minutes per PR, zero behaviour change.

## Phase 1: shrink the workspace (~2 weeks)

- **Remove `tono-desktop`.** Move to `marmikshah/tono-desktop` or archive the
  directory in history. Remove the WebKitGTK/AppIndicator/librsvg/patchelf
  installs from `ci-rust.yml`, the `tauri.conf.json` check from
  `release.py`, and the desktop prose in README, CONTRIBUTING, ADR 0001 and
  `docs/reference/determinism.md`. Lockfile drops from 528 to ≈260 packages.
- **Move content out of `tono-core`.** `library.rs`, `bgm.rs`, their tests,
  `site_samples.rs`, and the `tono-web` resource-budget test become
  `crates/tono-content` (unpublished, or published later as `tono-content`).
  Replace `bgm`'s private scale/instrument tables with `music::Scale` and
  `catalog` constructors; replace the `if calm/drive/else` triples in
  `bgm::score` with an arrangement table; replace `entry!`/`theme!` macros
  with struct literals. Decide whether to keep the parametric generators or
  commit the 420 generated JSON files and delete them.
- **Move `review.rs` into the CLI** (its only consumer) or delete it with
  the `review` subcommand.
- **Trim the CLI.** Replace hand-rolled parsing with `clap` (derive) for
  structured help, completions and man pages. Keep `generate`, `templates`,
  `render`, `vary`, `schema`, `compile`, `presets`, `catalog`, `play`.
  Delete `diff`, `match`, `fit`, `review`. Put `midi`/`import` behind a
  `midi` feature and FLAC/OGG behind an `encode` feature (default off;
  WAV stays). Drop `lib.rs`; move `site_samples.rs` to the content crate.
  Target: ≈2.2k lines, ≈55 deps, no C compile in the default build.
- **Publish `tono-play`** (or fold its 180-line `Speaker` into `tono-core`
  behind a `cpal` feature) so the CLI, Python and examples share one speaker
  adapter. Cut `tono-play` examples from 11 to 4 (`playground`, `live_band`,
  `adaptive`, `buses`).
- **Python examples:** keep the three API tours, move the seven compositions
  to the content repo or delete them.

Expected: −7k lines across the workspace, −270 lockfile packages, cold CI
loses its largest compile unit.

## Phase 2: engine 2.0 (~3–4 weeks, introduces engine revision 6)

This is the phase that justifies a major version. Order matters: do 2.1
first because it halves the cost of every later DSP change.

1. **One block-based evaluator.** Promote `streaming::{Src, Proc, Val}` to
   the only DSP implementation, with a block `process(&mut [f32], t0)` API
   so offline rendering does not regress to per-sample recursion. Offline
   `render` becomes build + one `fill(n)` + output stage. Delete
   `render/osc.rs` signal functions, `render/effects.rs` block kernels,
   `apply_processor`, `duck_envelope`, and the second pass of
   `render_tracks_impl`; move stats/stems accumulation into the stream
   mixer. Give `Proc::Biquad` a `Val` cutoff so the `ModulatedFilter`
   streaming blocker disappears. Whole-buffer nodes (convolve, granular,
   sampler seq) stay as explicit blockers. Gate: every golden hash and every
   `*_stream_byte_identically` test unchanged. ≈−1.3k lines.
2. **Curate the node vocabulary.** Keep the standard toolbox even though
   shipped content does not use it yet: `delay`, `compress`, `drive`,
   `chorus`, and the EQ family (`bandpass`, `notch`, `peak`, shelves).
   Remove or feature-gate the exotic and heavy ones: `convolve`, `granular`,
   `wavetable` morphing, `ringmod`, `bitcrush`/`downsample`, `flanger`,
   `phaser`, `duck` (replaced by sidechain on the mixer). Each removal drops
   its golden pin, fuzz strategy, docs row and Python name. This is the
   schema 3 / engine 6 change. ≈−1.5k lines.
3. **Consolidate the runtime.** `AdaptiveMusic` uses `Transport` for its
   clock and shares one `Scheduler<Action>` and one stinger/crossfade type
   with `Performance`; drop `adaptive::sections` in favour of
   `Performance::transition_to_section`. `Engine` stops re-rendering the
   whole document per `set_param`; instances move onto `StreamSource`, and
   `Ramp`/`Tween`/`gain_ramp` become one type. Collapse the four `play*`
   entry points into `spawn`. `Mixer` adopts the same pan law and reverb
   spread as document buses so a bus sounds the same either way. ≈−600 lines.
4. **Validation.** Split `validate_inner` (349 lines) into document and
   tracks validators; derive `vary::mutate_node` bounds and
   `song/diagnostics::validate_maps` from the same rule table as
   `dsl/validate.rs` so there is one place bounds live. Replace the
   hand-written per-node proptest strategies with one strategy derived from
   the schema. ≈−800 lines.
5. **Composition layer.** `compile_track` builds nodes directly instead of
   round-tripping `serde_json::json!`; `Program` stops duplicating `Song`
   fields in `ProgramMeta`. Keep the pattern algebra as is: Python binds all
   of it and it is proportionate.
6. **Test diet.** Table-drive streaming identity over a node corpus;
   collapse the timbre-character seq tests into one audibility table; keep
   golden, equivalence, rt_alloc, fuzz_time, fuzz_command_stream and soak
   untouched. ≈−1.4k lines.

Expected: `tono-core` ≈ 22–24k lines including tests (from 39k), one DSP
implementation, 25 node types, and a runtime with one clock and one
scheduler.

## Phase 3: public surface and release discipline (~1 week, parallel to Phase 2)

- **API freeze.** Review every `pub` item in `tono-core` (music.rs alone has
  64; three are used outside the crate). Default to `pub(crate)`. Publish a
  `prelude` that covers the documented flows and nothing else.
- **Semver in CI.** Add `cargo-semver-checks` against the last published
  version and a `cargo public-api` snapshot committed to the repo; a diff
  fails the PR unless labelled `breaking`.
- **Cross-platform CI.** Add macOS and Windows `cargo test -p tono-core
  -p tono` jobs. Determinism across platforms is the headline claim;
  `tests/golden.rs` on three OSes is the proof registries will look for.
  Restore the macOS/Windows CLI binaries in `release.yml` once green.
- **MSRV policy.** State it in README: pinned toolchain in
  `rust-toolchain.toml`, `rust-version` in Cargo, bumped only in minor
  releases with a changelog line. Add an MSRV job that builds with
  `rust-version`, not just the pinned channel.
- **Python.** Add a `stubtest`/pyright step to `wheel_test.py`; split
  `song.rs` into `song/pattern/program/instruments`; have core export the
  catalog slug table and processor-kind list so Python and the CLI stop
  mirroring them. Publish wheels to PyPI from `wheels.yml` on tags (the
  pipeline already exists and is tested).
- **Crate metadata.** `[package.metadata.docs.rs]`, badges (CI, crates.io,
  docs.rs, MSRV), `homepage` and `documentation` fields, keyword review.
  `tono-core`'s README is the workspace README; give it a short crate-specific
  one with the 15-line example and links.
- **Changelog.** Keep the release-parsed format; replace pre-1.8 history
  with a link to GitHub releases; rewrite "Unreleased" as the 2.0 notes with
  a migration section (removed nodes, removed subcommands, renamed items).
- **Governance files** already exist (CONTRIBUTING, CODE_OF_CONDUCT,
  SECURITY, templates). Add a `MAINTAINERS`/ownership note and a short
  "Versioning and stability" section to README.

## Phase 4: docs site (~1 week)

- Split the site. `tono` keeps: landing page, get-started lessons
  (`LearnSound` and its WASM audio stack stay; they are documentation),
  guides, reference (SoundDoc node table generated from `tono schema`, a real
  CLI reference generated from clap), ADRs, release process. A new
  `tono-site` or `tono-studio` repo takes the Sound Studio, the sound/BGM
  library browser, Showcase, the mp4s, `site_samples`, the `docs:audio`
  step and their node tests. Pages CI for the engine repo becomes
  `npm ci && vitepress build` plus the WASM build for the lessons.
- Merge duplicates: README release notes + `release-gates.md` +
  CONTRIBUTING "Releases" into one; README quickstart becomes
  `cargo install tono` + link; `docs/README.md` keeps only build
  instructions; `architecture.html` becomes a markdown page or goes.
- Drop `docs/project/performance.md` historical table; regenerate numbers
  from the criterion bench on the release machine when publishing.

## Sequencing and risk

| Order | Item | Risk | Gate |
|---|---|---|---|
| 1 | Phase 0 | none | CI green, golden unchanged |
| 2 | Remove desktop, CI trims | none | lockfile shrinks, CI time halves |
| 3 | Content crate, CLI trim | low (site build path changes) | `npm run docs:build` green |
| 4 | Phase 2.1 single evaluator | **medium**: mixer accumulation order, `peak_limit` baked gain, stats/stems | every golden hash identical; equivalence + streaming suites unchanged |
| 5 | Phase 2.2 node curation (engine 6) | low for shipped sound; schema change | new golden corpus for engine 6, old engine-5 docs rejected with a clear diagnostic |
| 6 | Phase 2.3–2.6 runtime/validation/tests | low | rt_alloc, fuzz_time, command-stream unchanged |
| 7 | Phase 3 API freeze + semver checks | low | `cargo public-api` snapshot committed |
| 8 | Cross-platform CI | unknown until run; `det.rs` was designed for it | golden on three OSes |
| 9 | Phase 4 site split | low | Pages green, site repo builds against published crates |
| 10 | Tag 2.0.0 | — | `bevy_tono` updated to 2.0 the same day |

Things not to do: do not rewrite the DSP math while unifying evaluators
(unify first, then change sound under engine 7 if ever); do not drop the
golden, equivalence or rt-alloc suites to make a refactor easier; do not
keep "just in case" node types or subcommands that have no documentation
and no user.

## Expected result

| | Today | After 2.0 |
|---|---|---|
| Rust lines (workspace) | ≈49k | ≈28–30k |
| `tono-core` lines | 39k | ≈22–24k |
| DSP implementations | 2 | 1 |
| Node types | 39 | ≈25 |
| Lockfile packages | 528 | ≈240 |
| CLI deps / C builds | 90 / 2 | ≈55 / 0 |
| CI minutes per PR | ≈20 | ≈8 across three OSes |
| Docs site code | 4.4k lines | ≈1k lines |
| Unverified API breakage | possible | caught by semver-checks |
