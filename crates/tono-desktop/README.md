# tono-desktop — the pattern station

The native desktop studio: a Tauri window with a step grid over catalog
instruments, live audio (cpal + a MIDI keyboard), and undo — authoring
SoundDocs by ear, with the deterministic engine underneath, so what you
audition is byte-identical to an offline bounce.

Not part of the default build or CI (webview/cpal/midir are heavy). Build and
launch:

```sh
make desktop
```
