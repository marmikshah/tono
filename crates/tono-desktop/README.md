# tono-desktop — the pattern station

A Tauri window over the audio engine: a step grid of catalog instruments,
live audition through `tono-play`, track mix/mute controls, undo/redo, and
saved song projects.

Build and launch from the workspace root:

```sh
cargo run --locked -p tono-desktop --release
```

Project loading validates the current format and grid. Unsupported or corrupt
saved data resets to a fresh project; read/write errors are shown in the UI.

The desktop is outside the default Cargo build and included in the full
Linux CI gate. Native dependencies and contributor commands are in the
[project README](../../README.md).
