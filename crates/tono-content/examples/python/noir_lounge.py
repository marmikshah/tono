"""noir_lounge — an original jazz-noir lounge piece in D minor (92 bpm):
rain on the window, a detective theme — vibraphone over a walking bass,
rootless Rhodes stabs, and a heavily swung ride cymbal.

The brief was "a detective theme". The idiom is public domain — minor
key, spang-a-lang ride, blue notes, b9 tension over the V — but no
existing score is, so every note here is an ORIGINAL melody written for
this example, not a transcription of anything.

Where golden_hour shows the producer side of the typed API, this one
shows the jazz-combo side: heavy swing on the drums and bass, a walking
quarter-note line built from `Pitch.transpose` arithmetic (root – third
– fifth – a stepwise or chromatic approach into the next root), extended
chords from `tono.Chord` (the 9th symbols don't parse, so the ninth is
added by Pitch arithmetic on the 7th-chord base), call-and-answer
phrasing between the vibes and the Rhodes, and a drum fade under the
final held Dm9.

Run with an installed tono wheel (or `maturin develop` in crates/tono-py):

    python3 crates/tono-content/examples/python/noir_lounge.py

It compiles the song, renders the mix, and writes `noir_lounge.wav`
(16-bit stereo) into the current directory — that's the track to listen to.
"""

import wave

import numpy as np
import tono

BPM = 92
BARS = 16

song = tono.Song("noir-lounge", tempo=BPM, seed=1947)

# --- The band -------------------------------------------------------------
drums = song.track("drums", tono.instruments.drums("acoustic").swing(0.3).humanize(0.1))
bass = song.track("bass", tono.instruments.bass("finger").gain(0.72).swing(0.3).pan(0.04))
keys = song.track("keys", tono.instruments.electric_piano("rhodes").gain(0.45).pan(-0.12).reverb(0.22).swing(0.3))
vibes = song.track("melody", tono.instruments.mallets("vibraphone").gain(0.65).pan(0.14).reverb(0.35))

# --- Harmony: A A B A, one chord per bar ------------------------------------
# D minor. The 9th symbols ("Dm9", "G9", "Cmaj9") don't parse in tono.Chord,
# so the chart maps onto the seventh-chord bases that do; the ninth itself
# is added by Pitch arithmetic where the voicings are built.
CHORDS = [
    "Dm9", "Dm9", "G9", "G9",        # a1 — the theme
    "Cmaj9", "A7", "Dm9", "A7",      # a2 — the answer, the V left hanging
    "G9", "Cmaj9", "A7", "Dm9",      # bridge
    "Dm9", "G9", "Cmaj9", "Dm9",     # last-a — home, and held
]
BASE = {"Dm9": "Dm7", "G9": "G7", "Cmaj9": "Cmaj7", "A7": "A7"}
ROOTS = {"Dm9": "D2", "G9": "G1", "Cmaj9": "C2", "A7": "A1"}

def stab(chord: str) -> list:
    """A rootless 3–5–7–9 voicing from the chart label: the seventh-chord
    tones straight from tono.Chord, the ninth from the root up a compound
    major second."""
    root, third, fifth, seventh = tono.Chord(BASE[chord]).pitches(3)
    return [third.name, fifth.name, seventh.name, root.transpose(14).name]

# --- Bass: the walk — one bar per chord, all Pitch arithmetic ----------------
def walk(chord: str, next_root: str, approach: int, gain: float = 0.9) -> tono.Pattern:
    """One walking bar: root – third – fifth – then beat 4 approaches the
    next bar's root from `approach` semitones away (stepwise or chromatic)."""
    root = tono.Pitch(ROOTS[chord])
    third = root.transpose(3 if "m7" in BASE[chord] else 4)
    fifth = root.transpose(7)
    lead = tono.Pitch(next_root).transpose(approach)
    p = tono.Pattern(bars=1)
    p.note(root.name, at=0.0, duration=0.9, gain=gain)
    p.note(third.name, at=1.0, duration=0.9, gain=gain * 0.92)
    p.note(fifth.name, at=2.0, duration=0.9, gain=gain * 0.95)
    p.note(lead.name, at=3.0, duration=0.9, gain=gain * 0.88)
    return p

# Beat 4's approach, in semitones from the NEXT bar's root: chromatic below
# (-1), the blue-side half step above (+1), or a whole step above (+2).
WALK = [
    ("Dm9", -1), ("Dm9", -1), ("G9", +2), ("G9", -1),      # a1
    ("Cmaj9", +2), ("A7", +1), ("Dm9", +2), ("A7", -1),    # a2
    ("G9", -1), ("Cmaj9", +2), ("A7", +1), ("Dm9", -1),    # bridge
    ("Dm9", -1), ("G9", -1), ("Cmaj9", -1),                # last-a
]
for bar, (chord, approach) in enumerate(WALK):
    song.arrange(bass, walk(chord, ROOTS[CHORDS[bar + 1]], approach), bars=bar)

# The last bar is held: the tonic under the fade.
last_bass = tono.Pattern(bars=1)
last_bass.note("D2", at=0.0, duration=3.75, gain=0.85)
song.arrange(bass, last_bass, bars=15)

# --- Drums: spang-a-lang ride, feathered kick, sparse comping -----------------
def kit(comp1=(), comp2=(), snare_vel: float = 0.52) -> tono.Pattern:
    """Two bars: ride on 1 &-2 3 &-4 (the swing voice bends the offbeats into
    the lilt), a feathered kick under 1 & 3, the hi-hat foot on 2 & 4, and a
    couple of offbeat snare stabs per bar."""
    ride = tono.Pattern(bars=2)
    ride.hit("ride", beats=[0, 0.5, 1, 2, 2.5, 3, 4, 4.5, 5, 6, 6.5, 7])
    soft = tono.Pattern(bars=2)
    soft.hit("kick", beats=[0, 2, 4, 6])
    soft.hit("hat", beats=[1, 3, 5, 7])
    snare = tono.Pattern(bars=2)
    if comp1:
        snare.hit("snare", beats=list(comp1))
    if comp2:
        snare.hit("snare", beats=[4 + b for b in comp2])
    return ride.vel(0.52).layer(soft.vel(0.3)).layer(snare.vel(snare_vel))

# Snare comping per 2-bar cell (bar one, bar two) — busier into the bridge,
# nearly nothing under the fade.
comps = [
    ((), (3.5,)), ((3.5,), (1.5,)),                    # a1
    ((1.5, 3.5), (2.5,)), ((3.5,), (1.5, 3.5)),        # a2
    ((2.5, 3.5), (1.5, 3.5)), ((1.5, 2.5), (3.5,)),    # bridge
    ((3.5,), (2.5,)), ((1.5,), ()),                    # last-a
]
for cell, (c1, c2) in enumerate(comps):
    song.arrange(drums, kit(c1, c2), bars=2 * cell)

# A last crash with the held chord, left to die in the fade.
crash = tono.Pattern(bars=1)
crash.hit("crash", beats=[0])
song.arrange(drums, crash.vel(0.55), bars=15)

# The drums own the fade: gain rides down across the last two bars.
song.automate(drums, "gain", [(0, 1.0), (56, 1.0), (64, 0.0)])

# --- The melody (original): bluesy vibes, more rests than notes ----------------
def phrase(bars: int, notes) -> tono.Pattern:
    p = tono.Pattern(bars=bars)
    for at, pitch, dur, gain in notes:
        p.note(pitch, at=at, duration=dur, gain=gain)
    return p

# a1 — the detective steps in: the long tonic, a turned figure (F–E–D); then
# over the G9 the blue third rubbing against the real one (Bb→B).
call_1 = phrase(2, [
    (0.00, "D5", 1.50, 0.78),
    (2.00, "F5", 0.75, 0.72), (2.75, "E5", 0.25, 0.66),
    (3.00, "D5", 1.00, 0.75),
    (5.00, "A4", 1.50, 0.70),
])
call_2 = phrase(2, [
    (0.50, "G4", 0.50, 0.68),
    (1.00, "Bb4", 0.75, 0.74), (1.75, "B4", 0.25, 0.66),
    (2.00, "D5", 1.50, 0.78),
    (4.50, "F5", 0.50, 0.72), (5.00, "D5", 0.50, 0.70),
    (5.50, "B4", 0.75, 0.74),
])

# a2 — C natural and Bb leaning on the A7 bars (the b9 and the rub).
call_3 = phrase(2, [
    (0.00, "E5", 1.00, 0.76), (1.50, "D5", 0.50, 0.70),
    (2.00, "C5", 1.50, 0.74),
    (4.50, "C5", 0.50, 0.70), (5.00, "Bb4", 1.25, 0.76),
])
call_4 = phrase(2, [
    (0.00, "D5", 2.00, 0.78),
    (4.00, "C5", 0.75, 0.70), (4.75, "Bb4", 0.25, 0.66),
    (5.00, "A4", 1.50, 0.74),
])

# bridge — up the G9 arpeggio, then the noir peak: Bb–C over the A7, home to D.
call_5 = phrase(2, [
    (0.50, "B4", 0.50, 0.70), (1.00, "D5", 1.00, 0.76),
    (2.50, "F5", 0.50, 0.72),
    (4.00, "E5", 1.50, 0.78), (5.50, "G5", 0.50, 0.74),
])
call_6 = phrase(2, [
    (0.00, "Bb4", 0.75, 0.78), (0.75, "C5", 0.25, 0.68),
    (1.00, "A4", 1.00, 0.74),
    (4.00, "D5", 2.00, 0.80),
])

# last-a — the theme recalled once, then the last word: C# leans into the
# final tonic, left to ring through the drum fade.
call_7 = phrase(2, [
    (0.00, "A4", 1.00, 0.72), (1.00, "D5", 1.50, 0.78),
    (3.00, "F5", 0.50, 0.72),
    (4.50, "Bb4", 0.50, 0.72), (5.00, "A4", 0.50, 0.70),
    (5.50, "G4", 0.75, 0.68),
])
call_8 = phrase(2, [
    (0.00, "E5", 1.00, 0.72), (1.00, "D5", 0.50, 0.68),
    (3.75, "C#5", 0.25, 0.62),
    (4.00, "D5", 3.50, 0.74),
])

# A loose, late-night hand on the vibes — baked in, deterministic, a fixed
# seed per phrase.
calls = [call_1, call_2, call_3, call_4, call_5, call_6, call_7, call_8]
calls = [p.humanize(timing=0.2, velocity=0.08, seed=401 + i) for i, p in enumerate(calls)]
for i, p in enumerate(calls):
    song.arrange(vibes, p, bars=2 * i)

# --- Keys: rootless stabs on swung offbeats, and the answers -------------------
def comp(chord: str, on_beats=(2.5, 3.5), gain: float = 0.6) -> tono.Pattern:
    """Two short stabs on the swung offbeats, rootless 3–5–7–9."""
    p = tono.Pattern(bars=1)
    p.chord(stab(chord), at=on_beats[0], duration=0.5, gain=gain)
    p.chord(stab(chord), at=on_beats[1], duration=0.25, gain=gain * 0.85)
    return p

def answer(notes, stab_chord=None) -> tono.Pattern:
    """The Rhodes answers in the gap the vibes leave — the call's tail an
    octave down, with a stab on the downbeat where the chord changes."""
    p = tono.Pattern(bars=1)
    if stab_chord:
        p.chord(stab(stab_chord), at=0.0, duration=1.0, gain=0.55)
    for at, pitch, dur, gain in notes:
        p.note(pitch, at=at, duration=dur, gain=gain)
    return p

# Comping bars (the first bar of each cell), alternating the figure.
song.arrange(keys, comp("Dm9"), bars=0)
song.arrange(keys, comp("G9", on_beats=(1.5, 3.5)), bars=2)
song.arrange(keys, comp("Cmaj9"), bars=4)
song.arrange(keys, comp("Dm9", on_beats=(1.5, 3.5)), bars=6)
song.arrange(keys, comp("G9"), bars=8)
song.arrange(keys, comp("A7", on_beats=(1.5, 3.5)), bars=10)
song.arrange(keys, comp("Dm9"), bars=12)
song.arrange(keys, comp("Cmaj9", on_beats=(1.5, 3.5)), bars=14)

# Answer bars (the second bar of each cell) — echoing each call's tail.
song.arrange(keys, answer([(2.50, "F4", 0.50, 0.60), (3.00, "E4", 0.25, 0.55),
                           (3.50, "D4", 0.50, 0.58)]), bars=1)
song.arrange(keys, answer([(2.50, "Bb3", 0.50, 0.60), (3.00, "B3", 0.25, 0.55),
                           (3.50, "D4", 0.50, 0.58)]), bars=3)
song.arrange(keys, answer([(2.50, "C#4", 0.50, 0.60), (3.50, "Bb3", 0.50, 0.58)],
                          stab_chord="A7"), bars=5)
song.arrange(keys, answer([(2.50, "G3", 0.50, 0.58), (3.00, "Bb3", 0.25, 0.55),
                           (3.50, "C#4", 0.50, 0.60)], stab_chord="A7"), bars=7)
song.arrange(keys, answer([(2.50, "E4", 0.50, 0.60), (3.00, "G4", 0.50, 0.55)],
                          stab_chord="Cmaj9"), bars=9)
song.arrange(keys, answer([(2.50, "F4", 0.50, 0.62), (3.00, "E4", 0.25, 0.55),
                           (3.50, "D4", 0.50, 0.58)], stab_chord="Dm9"), bars=11)
song.arrange(keys, answer([(2.50, "Bb3", 0.50, 0.60), (3.00, "A3", 0.25, 0.55),
                           (3.50, "G3", 0.50, 0.58)], stab_chord="G9"), bars=13)

# The held Dm9 under the fade — the chord the whole band lands on.
held = tono.Pattern(bars=1)
held.chord(stab("Dm9"), at=0.0, duration=3.75, gain=0.55)
song.arrange(keys, held, bars=15)

# --- Rides and the map ---------------------------------------------------------
song.add_section("a1", bar=0, bars=4)
song.add_section("a2", bar=4, bars=4)
song.add_section("bridge", bar=8, bars=4)
song.add_section("last-a", bar=12, bars=4)
song.add_marker("bridge", beat=32)

# --- Compile, render, bounce --------------------------------------------------
program = song.compile(sample_rate=48_000)
print(f"compiled  hash={hex(program.hash)}")
print(f"          {BARS} bars of 4/4 at {BPM} bpm = "
      f"{program.duration_seconds:.1f}s, streamable={program.is_streamable}")
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
with wave.open("noir_lounge.wav", "wb") as w:
    w.setnchannels(2)
    w.setsampwidth(2)
    w.setframerate(program.sample_rate)
    w.writeframes(pcm.tobytes())
print("wrote     noir_lounge.wav — press play.")
