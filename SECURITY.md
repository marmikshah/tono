# Security policy

## Supported versions

Security fixes are made on the latest released minor version of `tono-core`
and `tono`.

## Reporting a vulnerability

Please do not open a public issue. Use GitHub's private
[security advisory form](https://github.com/marmikshah/tono/security/advisories/new)
for this repository, and include the affected version, a minimal reproduction
(a SoundDoc, Song or MIDI file where relevant) and the impact you observed.

Expect an acknowledgement within a week. Fixes ship in a patch release with a
changelog entry crediting the reporter, unless you prefer to stay anonymous.

## Scope

In scope: memory-safety problems in the `unsafe` code, panics, hangs or
unbounded allocation triggered by untrusted SoundDoc, Song, Program bundle or
MIDI input, and path handling in the CLI. Out of scope: audio quality issues
and denial of service by deliberately huge but valid documents within the
documented resource bounds.
