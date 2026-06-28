---
name: sound-designer
description: Professional sound-design methodology for working with sonarium MCP tools. Use when authoring, polishing, mixing, or reviewing sounds, SFX, or music through sonarium (author_sound, analyze, set_param, compare_sounds...) — encodes the listen-and-fix loop, how to read every analysis metric and both feedback images, numeric targets per SFX archetype, symptom→fix recipes, and the ship checklist.
---

# Sound Designer

You are working a session like a sound designer at a DAW: every render hands
back numbers and two images. Never judge a sound by its graph — judge it by
its analysis, and let every edit be a hypothesis the next render confirms.

## The loop

1. **Author** the smallest graph that could work (`author_sound`). For layered
   work, author the BODY first, then stack (`layer { op: "add" }` where available,
   `mix` inputs otherwise).
2. **Read the numbers, then look at both images.** Numbers tell you *what*;
   the spectrogram tells you *where in frequency*; the waveform tells you
   *where in time*.
3. **Make ONE targeted edit** (`set_param` — path-addressed; call
   `describe_sound` first). Whole-graph `refine_sound` only for structural
   rework.
4. **Compare** (`compare_sounds` old vs new, or re-read the stats). If worse,
   `history { op: "undo" }` — never pile a fix on a regression.
5. **Stop when the archetype targets are met** (below), not when the graph
   looks clever. Over-iteration past the targets usually trades character for
   conformity.

## Reading the analysis

| Metric | It means | Healthy | When off |
|---|---|---|---|
| `peak_dbfs` | sample ceiling | −6..−0.1 | < −12: raise gain or `normalize`; ≈ 0 with low RMS: one rogue transient |
| `rms_dbfs` / `loudness_lufs` | perceived level | SFX pack: −16 LUFS; BGM: −14 | mismatched across a set: export with `target_lufs`, don't hand-tune |
| `crest_factor_db` | punchiness (peak−RMS) | percussive 12–20; sustained 6–10 | < 6 on a hit: add `punch`, shorten attack, ease compression; > 20 on a bed: compress |
| `spectral_centroid_hz` | brightness | see archetypes | too dark: raise cutoff / brighter wave (square, saw); too harsh: lowpass, lower duty, less drive |
| `attack_time_ms` | transient speed | hits < 10; swells 100+ | slow hit: `env.a: 0`, add `punch`; check head silence isn't eating it |
| `decay_time_ms` | ring-out | UI < 150; impacts 200–600; pads 1000+ | clipped tail: raise `r`/duration; endless: shorten `d`, lower reverb `mix` |
| `onset_count` | distinct attacks | 1 for one-shots | > 1 unintended: overlapping notes, double trigger, or chorus/delay re-attack |
| `head_silence_ms` | dead air before | < 10 | trim with `env.a`, note `step`, or layer `at` |
| `tail_silence_ms` | dead air after | < 100 | shorten `duration` — dead air ships as file size and latency |
| `layers[]` | per-layer balance (post-fader, pre-master) | no layer > ~70% unless intentional | fix balance with the named layer's fader, not by re-EQing everything |

## Reading the images

- **Spectrogram**: harmonic stacks = pitched tone; broadband wash = noise;
  a hard horizontal band that never moves = resonance to notch; energy packed
  below ~500 Hz with a quiet top = mud; descending comb = your slide working.
- **Waveform**: the attack edge should be a wall for hits, a ramp for swells;
  flat-topped blocks = clipping/over-limiting; bumps after the hit =
  double-trigger; a long thin tail = trim candidate; for loops the two ends
  should meet at similar amplitude.

## Archetype targets

| Archetype | Duration | Attack | Centroid | Crest | The trick |
|---|---|---|---|---|---|
| Laser / zap | 0.1–0.4 s | < 5 ms | 2–6 kHz, falling | > 12 dB | exp slide down + 3–5 ms noise transient layer |
| Coin / pickup | 0.3–0.9 s | < 10 ms | 1.5–4 kHz | 8–14 dB | two pure blips, 4th or 5th apart, second held |
| Jump | 0.2–0.4 s | < 10 ms | rising | 8–14 dB | exp slide UP, fast decay gate |
| Explosion / impact | 0.5–2 s | < 10 ms | 200–900 Hz | 10–16 dB | noise → falling lowpass; sub thump layer; ring tail |
| UI click / confirm | 0.05–0.25 s | < 5 ms | 2–6 kHz | > 12 dB | one tiny blip; confirm = two, error = minor 2nd |
| Ambience / bed | loop 5–30 s | n/a | < 1.5 kHz | < 8 dB | filtered noise + slow LFOs; `make_loop`, seam < −40 dB |
| BGM / band | bars | n/a | mix-dependent | 8–12 dB | `tracks` mixer; kit + bass center, leads panned; master compress + reverb |

## Symptom → fix

- **Muddy** (centroid low, energy 250–500 Hz): highpass 80–150 Hz, or cut the
  offending layer's gain — check `layers[]` for who owns the mud.
- **Harsh / fizzy** (centroid high, top hurts): lowpass 6–9 kHz, duty toward
  0.5, less `drive`; on seqs prefer `triangle`/`sine` for highs.
- **Weak / no punch**: `env.punch` 0.2–0.4, `a: 0`, crest check; a
  `compress` with makeup AFTER the transient layer, never before.
- **Clicks at note ends**: `r` ≥ 0.01 s; at loop seams: bigger
  `crossfade_secs`.
- **Thin** (no body): add a sine/triangle layer an octave down at ~0.5 gain,
  or a sub layer for impacts.
- **Static / lifeless** (sustained sounds): LFO on cutoff or duty
  (rate 0.2–2 Hz, shallow), or `humanize` for repeats.
- **Quiet next to siblings**: never fix in the graph — `export` /
  `export_pack` with one `target_lufs` for the whole set.

## Layered sounds (schema v2)

One layer per thing you'd fade, pan, time-shift, or analyze separately —
transient / body / tail for SFX, one instrument per layer for music. Address
layers by id (`layer { op: "set" }` for gain/pan/at/mute; `set_param {layer, path}` for
the instrument). Read the `layers (post-fader, pre-master)` line every render:
balance moves beat EQ surgery. On older servers without layer tools, the same
design maps to `mix` inputs — keep components as separate inputs so paths
stay addressable.

## Ship checklist (before export / bank)

- [ ] `peak_dbfs` ≤ −0.1, no flat-topped waveform
- [ ] set loudness-matched: one `target_lufs` per pack (−16 SFX / −14 BGM)
- [ ] `head_silence_ms` < 10 (unless a deliberate pre-delay)
- [ ] `tail_silence_ms` < 100 — trim `duration` otherwise
- [ ] `onset_count` matches intent
- [ ] loops: seam < −40 dB, `playback: loop` so the WAV carries the `smpl` chunk
- [ ] names are engine-asset-ready slugs; round-robin sets via
  `generate_variants`/`humanize`, grouped with `rr_group` in the bank
