# Release workflow

Builds, CI, CLI binaries, and wheels are Linux-only. Windows/macOS patches are
welcome, but those builds are no longer funded. Current document formats are
SoundDoc/Song 2, DSP engine 5, and Program 3.

## Prepare a release

Update the unreleased entries in `CHANGELOG.md` and set their version/date when
the release is ready. Preserve older entries as release history. Bump both the
workspace package version and the `tono-core` dependency version in `Cargo.toml`;
all member crates inherit that version.

With Python 3.11+, run `python scripts/release.py check-version vX.Y.Z`.
Run the root README's test/lint commands, strict Rust documentation, the
ignored deterministic soaks, and `cargo deny check advisories licenses`.
For performance-sensitive changes, compare the [benchmarks](performance.md)
on the same machine and investigate PCM or command-trace changes.

Ready PRs run the reusable `.github/workflows/ci-rust.yml` gate with pinned
1.88.0 and latest Rust: every crate/feature, lean core configurations, native
release builds, installed-wheel tests, and minimum Python 3.9 coverage.
The Pages workflow checks TypeScript, the production site, and npm advisories.
Review the results on the commit being released; local checks complement them.

## Publish the reviewed commit

Tag the reviewed commit on `master` as `vX.Y.Z` and push the tag when the release
is authorized. `.github/workflows/publish.yml` uses the `production` environment
and `CRATES_API_TOKEN`: publish core first, wait for its registry index entry,
then publish the CLI. Already-published versions are skipped on a retry.

A CLI package dry run requires its matching core version to be in the registry.
Before publication, `cargo publish --locked -p tono-core --dry-run` checks the
core package. No automated rollback can remove a published crate version.

The tag release workflow reads that version's changelog section and uploads a
Linux x86_64 CLI binary and SHA-256 sidecar to the GitHub Release. Missing release
notes fail the workflow rather than create an empty release body.
Python wheels use a separate manual workflow with the same
version check, manylinux build, isolated install tests, and checksums; it does
not publish to PyPI. Retained packaging scripts document arguments in `--help`.

Physical audio, an interactive desktop window, and real SoundFont playback need
a suitable host and device. Software soak and headless tests do not replace
those checks; record any unavailable verification in the release review.
