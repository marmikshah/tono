---
name: sound-review-loop
description: Run an iterative Review → Polish → Review loop on a tono sound until it meets its archetype targets (or the user stops). Use when asked to "review and polish a sound", "iterate on this sound", "run a polish/review loop", "keep improving until it passes", or to set up a sound-review system. Review comes from the deterministic `review_sound` tool by default; the user can supply review in their own words at any iteration and it takes over.
---

# Sound Review Loop

A closed loop that drives a sound toward its targets: **Review → Polish →
Review**, repeating until it passes or the user calls it. The review is
reproducible (the `review_sound` tool grades against archetype targets + the
ship checklist), the polish is yours (one targeted edit per turn), and a human
can step in as the reviewer whenever they want.

Pair this with the **sound-designer** skill — it owns the metric reading,
symptom→fix recipes, and archetype targets this loop acts on.

## Setup

You need:
- a **sound id** (author or `scaffold_layered_sfx` one first if there isn't one),
- an **archetype** — `laser` / `coin` / `jump` / `impact` / `ui` / `ambience` /
  `bgm` (ask if it's ambiguous; omit only for a generic clip/level/seam check),
- optional **max iterations** (default **5**) and an optional **reference**
  sound id to converge toward with `compare_sounds`.

## The loop

Repeat until a stop condition (below):

1. **REVIEW.**
   - If the user gave feedback *this turn*, that IS the review — human review
     overrides the tool for this iteration (see Human review).
   - Otherwise call `review_sound { id, archetype }`. It returns a `grade`
     (PASS/WARN/FAIL), per-criterion `findings`, and for each non-pass finding
     a `fix` to try. If a reference was given, also read `compare_sounds`.
2. **DECIDE.** Stop if `grade == PASS` (and the user hasn't asked for more), or
   the user said it's good, or you hit max iterations. Otherwise continue.
3. **POLISH — exactly ONE edit.** Take the **highest-severity** finding (FAIL
   before WARN; if tied, the most audible: clipping/onset > crest/centroid >
   silence). Apply its `fix` as a single `set_param` / `edit_sound`
   (`describe_sound` first for the path). One hypothesis per turn — never batch
   fixes, or you can't tell which one worked.
4. **RE-REVIEW.** Call `review_sound` again. If the grade got **worse**, or the
   finding you targeted regressed, **`history { op: "undo" }`** and try a different fix —
   never pile a fix on a regression.
5. Report the turn in one line: `iter N: <finding> → <edit> → grade X→Y
   (metric a→b)`. Then loop.

## Human review (the override)

When the user speaks during the loop, treat their words as the authoritative
review for that iteration, above the tool:
- Map their language to edits via the sound-designer **symptom→fix** table —
  "too harsh" → lowpass / less drive; "needs punch" → `env.punch`, shorter
  attack; "too long" → trim duration; "more body" → sub/octave layer.
- "Looks/sounds good", "ship it", "that's it" → **stop the loop**, report final
  grade, suggest `export` (or `export_pack` with one `target_lufs`).
- Their judgment wins even if `review_sound` still shows a WARN — a passing
  meter never overrides a human "good enough", and a human "not yet" keeps the
  loop going past a PASS.

## Stop conditions

- `grade == PASS` and no outstanding user request → **done**.
- **max iterations** reached → stop, report the remaining findings, ask whether
  to continue or accept.
- The only findings left are **WARNs the sound's character justifies** (e.g. a
  gusting wind's crest is "high" for a bed, a bell's tail is "long") → stop and
  SAY SO. Do not chase a target into conformity — over-iteration past the
  targets trades character for spec (the sound-designer rule).
- Two iterations with no grade improvement → stop and surface the wall to the
  user rather than thrashing.

## Running it

- **Interactive** (default): one iteration per turn, pause for the user to
  inspect the images and optionally review. Good when taste matters.
- **Autonomous**: run iterations back-to-back until a stop condition; use the
  `/loop` skill to self-pace if the user wants it unattended. Always honour max
  iterations as the backstop.

## Worked shape

```
author/scaffold → review_sound{laser} → FAIL (crest 7, centroid 1200)
  iter1: crest FAIL → add a 3 ms noise transient layer → review → WARN (crest 10→13 pass, centroid still low)
  iter2: centroid WARN → raise the slide's start freq 1200→2200 → review → PASS
done: PASS (7 pass, 0 warn). suggest export at -16 LUFS.
```
