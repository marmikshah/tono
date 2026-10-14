"""golden_hour — a produced 16-bar piece, composed end to end with the typed
tono API. The sound-designer counterpart to night_drive's feature tour: this
one is about the MUSIC — a boom-bap pocket, a i–VI–III–v progression, a
call-and-answer vibraphone melody, and the production touches (swing,
humanize, reverb bus, gain rides) that make it feel finished.

Run with an installed tono wheel (or `maturin develop` in crates/tono-py):

    python3 crates/tono-content/examples/python/golden_hour.py

It compiles the song, renders the mix, and writes `golden_hour.wav` (16-bit
stereo) into the current directory — that's the track to listen to.
"""

import wave

import numpy as np
import tono

BPM = 92
BARS = 16

song = tono.Song("golden-hour", tempo=BPM, seed=42)

# --- The band -------------------------------------------------------------
drums = song.track("drums", tono.instruments.drums("tr808").swing(0.22).humanize(0.15))
bass = song.track("bass", tono.instruments.bass("sub").swing(0.22).gain(0.9))
keys = song.track("keys", tono.instruments.electric_piano("rhodes").pan(-0.12).reverb(0.22))
vibes = song.track("melody", tono.instruments.mallets("vibraphone").pan(0.18).humanize(0.1))
strings = song.track("strings", tono.instruments.strings("ensemble"))

# A shared room for keys and melody — a real mix bus, not a per-track insert.
song.add_bus("room", gain=0.9, effects=[("reverb", {"room": 0.45, "mix": 0.22})])
keys.send("room", 0.25)
vibes.send("room", 0.4)

# --- Harmony: Am7 — Fmaj7 — Cmaj7 — Em7 (i–VI–III–v), two bars each --------
prog = [[p.name for p in tono.Chord(c).pitches(3)] for c in ("Am7", "Fmaj7", "Cmaj7", "Em7")]
roots = ["A2", "F2", "C3", "E2"]  # sub-bass roots, one per chord
# Where each chord holds, in bars (intro → groove → lift → resolve home).
where = {
    0: [0, 1, 4, 5, 15],  # Am7 — home, and the last word
    1: [2, 3, 6, 7, 12, 13],  # Fmaj7 — the lift leans on it
    2: [8, 9],            # Cmaj7
    3: [10, 11, 14],      # Em7 — the minor dominant pull
}

# --- Keys: held chord on the downbeat, two laid-back stabs after ------------
def keys_pattern(voicing) -> tono.Pattern:
    p = tono.Pattern(bars=1)
    p.chord(voicing, at=0, duration=1.75)
    p.chord(voicing, at=2.5, duration=0.75, gain=0.8)
    p.chord(voicing, at=3.5, duration=0.5, gain=0.7)
    return p

for i, bars in where.items():
    song.arrange(keys, keys_pattern(prog[i]), bars=bars)

# --- Bass: root – root – fifth – octave, locked to the kick ----------------
# The bass sits OUT of the intro — its entrance with the drums is the drop.
bass_where = {
    0: [4, 5],              # Am7, plus the outro bar below
    1: [6, 7, 12, 13],      # Fmaj7
    2: [8, 9],              # Cmaj7
    3: [10, 11, 14],        # Em7
}

def bass_line(root: str) -> tono.Pattern:
    fifth = tono.Pitch(root).transpose(7).name
    octave = tono.Pitch(root).transpose(12).name
    p = tono.Pattern(bars=2)
    p.note(root, at=0, duration=1.5)
    p.note(root, at=2, duration=0.5)
    p.note(fifth, at=2.75, duration=0.25)
    p.note(octave, at=3.5, duration=0.5)
    # Bar two answers with a step back down to the root.
    p.note(root, at=4, duration=1.5)
    p.note(root, at=6, duration=0.75)
    p.note(fifth, at=7, duration=0.5)
    p.note(root, at=7.5, duration=0.5)
    return p

for i, bars in bass_where.items():
    # Two bars per placement — pair up consecutive bars and place at the run
    # start (placing a 2-bar pattern at every bar would stack it double).
    for b in bars:
        if b + 1 in bars:
            song.arrange(bass, bass_line(roots[i]), bars=b)
# Single bars (the lift's Em7, the outro tonic) take the figure's first bar,
# the outro at half level so the ending breathes.
song.arrange(bass, bass_line("E2").slice(0, 16), bars=14)
song.arrange(bass, bass_line("A2").slice(0, 16).vel(0.5), bars=15)

# --- Drums: boom-bap kick, snare on 2 & 4, swung hats -----------------------
groove = tono.Pattern(bars=1)
groove.hit("kick", beats=[0, 1.75, 2])
groove.hit("snare", beats=[1, 3])
groove.hit("hat", beats=[x * 0.5 for x in range(8)])

fill = tono.Pattern(bars=1)
fill.hit("kick", beats=[0, 1.75])
fill.hit("snare", beats=[1, 3, 3.5, 3.75])
fill.hit("openhat", beats=[3.5])

crash = tono.Pattern(bars=1)
crash.hit("crash", beats=[0])

# The groove runs bars 4–14 with a fill every fourth bar — and one into the
# outro; the outro itself is bare.
for bar in range(4, 15):
    song.arrange(drums, fill if bar % 4 == 3 or bar == 14 else groove, bars=bar)
song.arrange(drums, crash, bars=[4, 12])
# Enter slightly under, crown the lift, slip away for the outro.
song.automate(drums, "gain", [(16, 0.85), (48, 1.0), (60, 0.75)])

# --- Melody: A-minor pentatonic, call and answer on vibraphone --------------
call = tono.Pattern(bars=2)
call.note("E5", at=0.5, duration=0.5)
call.note("D5", at=1.0, duration=0.25)
call.note("C5", at=1.5, duration=0.5)
call.note("A4", at=2.0, duration=1.5)
call.note("G4", at=4.5, duration=0.5)
call.note("A4", at=5.0, duration=1.0)
call.note("C5", at=6.5, duration=0.5)
call.note("D5", at=7.0, duration=0.75)

answer = tono.Pattern(bars=2)
answer.note("E5", at=0.5, duration=0.5)
answer.note("G5", at=1.0, duration=0.75)
answer.note("E5", at=2.0, duration=0.5)
answer.note("D5", at=2.5, duration=0.5)
answer.note("C5", at=3.0, duration=1.5)
answer.note("D5", at=5.5, duration=0.5)
answer.note("C5", at=6.0, duration=0.5)
answer.note("A4", at=6.5, duration=1.25)

# A light human hand on the vibes only — baked in, deterministic.
call = call.humanize(timing=0.35, velocity=0.08, seed=11)
answer = answer.humanize(timing=0.35, velocity=0.08, seed=12)

song.arrange(vibes, call, bars=[0, 4, 8])
song.arrange(vibes, answer, bars=[2, 6, 10])
song.arrange(vibes, call.transpose(12), bars=12)   # the lift, an octave up
song.arrange(vibes, answer.transpose(12), bars=14)
# The last word: a single held tonic over the outro chord.
last = tono.Pattern(bars=1)
last.note("A5", at=0, duration=3.5, gain=0.9)
song.arrange(vibes, last, bars=15)

# --- Strings: the lift's pad, faded in under the melody ---------------------
def pad(voicing) -> tono.Pattern:
    p = tono.Pattern(bars=1)
    p.chord(voicing, at=0, duration=4, gain=0.7)
    return p

song.arrange(strings, pad(prog[1]), bars=[12, 13])
song.arrange(strings, pad(prog[3]), bars=14)
song.arrange(strings, pad(prog[0]), bars=15)
song.automate(strings, "gain", [(48, 0.0), (50, 0.85), (63, 0.7)], curve="exp")

# --- Rides and the map -------------------------------------------------------
song.automate(keys, "gain", [(0, 0.7), (16, 0.9), (63, 0.65)], curve="exp")
song.add_section("intro", bar=0, bars=4)
song.add_section("groove", bar=4, bars=8)
song.add_section("lift", bar=12, bars=4)
song.add_marker("first-kick", beat=16)

# --- Compile, render, bounce -------------------------------------------------
program = song.compile(sample_rate=48_000)
print(f"compiled  hash={hex(program.hash)}")
print(f"          {BARS} bars at {BPM} bpm = {program.duration_seconds:.1f}s, "
      f"streamable={program.is_streamable}")
print(f"          estimates: {program.estimates}")
if program.warnings:
    print(f"          warnings: {program.warnings}")

mix = program.render()
peak = float(abs(mix).max())
print(f"rendered  {mix.shape[0]} frames stereo, peak {peak:.3f} "
      f"({20 * np.log10(peak):.1f} dBFS)")
print(f"stems     {sorted(program.render_stems())}")

# A plain 16-bit WAV via the stdlib — no extra dependencies.
pcm = (mix.clip(-1.0, 1.0) * 32767).astype("<i2")
with wave.open("golden_hour.wav", "wb") as w:
    w.setnchannels(2)
    w.setsampwidth(2)
    w.setframerate(program.sample_rate)
    w.writeframes(pcm.tobytes())
print("wrote     golden_hour.wav — press play.")
