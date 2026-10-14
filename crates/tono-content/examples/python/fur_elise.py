"""fur_elise — Beethoven's Bagatelle No. 25 in A minor, WoO 59 ("Für Elise",
1810): the famous opening section, played on the sampled grand. The
composition is public domain; the note data follows the Breitkopf & Härtel
1888 edition (via the Mutopia Project, also public domain).

Where golden_hour shows the producer side of the typed API, this one shows
the classical side: a real 3/8 meter map with an eighth-note pickup, phrase
patterns arranged on true bar boundaries, a ritardando on the tempo map, and
per-note dynamics (the edition is marked pp — the left hand whispers).

Run with an installed tono wheel (or `maturin develop` in crates/tono-py):

    python3 crates/tono-content/examples/python/fur_elise.py

It compiles the song, renders the mix, and writes `fur_elise.wav` (16-bit
stereo) into the current directory — that's the track to listen to.
"""

import wave

import numpy as np
import tono

BPM = 72  # quarter note — the edition's poco moto marking

song = tono.Song("fur-elise", tempo=BPM, seed=1770)

# The piece is genuinely 3/8 with a two-sixteenth anacrusis — say so, and
# bars/sections/MIDI export all line up with the printed score.
song.set_meter_map([(0, 3, 8)])
song.set_pickup(0.5)

piano = song.track("piano", tono.instruments.piano("grand").gain(1.35).reverb(0.28))

# --- Dynamics: poco moto, pianissimo ----------------------------------------
HELD = 0.9    # the long melody note on each downbeat
MELODY = 0.82  # the running sixteenths
FILL = 0.62    # right-hand alto fills tucked under the melody
BASS = 0.5     # the left hand's broken chords

# --- The score ---------------------------------------------------------------
# Notes as (at, pitch, duration, gain) in beats, phrase by phrase. A 3/8 bar
# is 1.5 beats; the pickup is beat 0 of bar 0, so bar N starts at
# 0.5 + (N - 1) * 1.5. The phrases below tile that lattice exactly.

def phrase(bars: int, notes) -> tono.Pattern:
    p = tono.Pattern(bars=bars)
    for at, pitch, dur, gain in notes:
        p.note(pitch, at=at, duration=dur, gain=gain)
    return p

# Pickup + bars 1–4: the question, and its first answer.
theme_a = phrase(2, [
    (0.0, "E5", 0.25, MELODY), (0.25, "D#5", 0.25, MELODY),
    (0.5, "E5", 0.25, MELODY), (0.75, "D#5", 0.25, MELODY), (1.0, "E5", 0.25, MELODY),
    (1.25, "B4", 0.25, MELODY), (1.5, "D5", 0.25, MELODY), (1.75, "C5", 0.25, MELODY),
    (2.0, "A4", 0.5, HELD), (2.75, "C4", 0.25, FILL), (3.0, "E4", 0.25, FILL), (3.25, "A4", 0.25, FILL),
    (3.5, "B4", 0.5, HELD), (4.25, "E4", 0.25, FILL), (4.5, "G#4", 0.25, FILL), (4.75, "B4", 0.25, FILL),
    (5.0, "C5", 0.5, HELD), (5.75, "E4", 0.25, FILL), (6.0, "E5", 0.25, MELODY), (6.25, "D#5", 0.25, MELODY),
    # Left hand: broken-chord whispers under the held notes.
    (2.0, "A2", 0.25, BASS), (2.25, "E3", 0.25, BASS), (2.5, "A3", 0.25, BASS),
    (3.5, "E2", 0.25, BASS), (3.75, "E3", 0.25, BASS), (4.0, "G#3", 0.25, BASS),
    (5.0, "A2", 0.25, BASS), (5.25, "E3", 0.25, BASS), (5.5, "A3", 0.25, BASS),
])

# Bars 5–8: the question again, cadencing up into the episode (B–C–D lead).
theme_b = phrase(2, [
    (0.0, "E5", 0.25, MELODY), (0.25, "D#5", 0.25, MELODY), (0.5, "E5", 0.25, MELODY),
    (0.75, "B4", 0.25, MELODY), (1.0, "D5", 0.25, MELODY), (1.25, "C5", 0.25, MELODY),
    (1.5, "A4", 0.5, HELD), (2.25, "C4", 0.25, FILL), (2.5, "E4", 0.25, FILL), (2.75, "A4", 0.25, FILL),
    (3.0, "B4", 0.5, HELD), (3.75, "E4", 0.25, FILL), (4.0, "C5", 0.25, FILL), (4.25, "B4", 0.25, FILL),
    (4.5, "A4", 0.5, HELD), (5.25, "B4", 0.25, 0.7), (5.5, "C5", 0.25, 0.75), (5.75, "D5", 0.25, 0.8),
    (1.5, "A2", 0.25, BASS), (1.75, "E3", 0.25, BASS), (2.0, "A3", 0.25, BASS),
    (3.0, "E2", 0.25, BASS), (3.25, "E3", 0.25, BASS), (3.5, "G#3", 0.25, BASS),
    (4.5, "A2", 0.25, BASS), (4.75, "E3", 0.25, BASS), (5.0, "A3", 0.25, BASS),
])

# Bars 9–13: the rocking episode — a long melody note, a dip below, answer;
# then the bell-like high-octave answer everyone waits for.
episode = phrase(2, [
    (0.0, "E5", 0.75, 0.9), (0.75, "G4", 0.25, 0.66), (1.0, "F5", 0.25, 0.78), (1.25, "E5", 0.25, 0.82),
    (1.5, "D5", 0.75, 0.88), (2.25, "F4", 0.25, 0.64), (2.5, "E5", 0.25, 0.76), (2.75, "D5", 0.25, 0.8),
    (3.0, "C5", 0.75, 0.86), (3.75, "E4", 0.25, 0.62), (4.0, "D5", 0.25, 0.74), (4.25, "C5", 0.25, 0.78),
    (4.5, "B4", 0.5, 0.84), (5.25, "E4", 0.25, 0.6), (5.5, "E5", 0.25, 0.7),
    (6.25, "E5", 0.25, 0.6), (6.5, "E6", 0.25, 0.66), (7.25, "D#5", 0.25, 0.62),
    (0.0, "C3", 0.25, BASS), (0.25, "G3", 0.25, BASS), (0.5, "C4", 0.25, BASS),
    (1.5, "G2", 0.25, BASS), (1.75, "G3", 0.25, BASS), (2.0, "B3", 0.25, BASS),
    (3.0, "A2", 0.25, BASS), (3.25, "E3", 0.25, BASS), (3.5, "A3", 0.25, BASS),
    (4.5, "E2", 0.25, BASS), (4.75, "E3", 0.25, BASS), (5.0, "E4", 0.25, BASS),
])

# Bar 14: the dominant gathers itself for the return.
return_lead = phrase(1, [
    (0.0, "E5", 0.5, 0.86), (0.75, "D#5", 0.25, 0.72), (1.0, "E5", 0.25, 0.78), (1.25, "D#5", 0.25, 0.72),
    (0.0, "E2", 0.25, BASS), (0.25, "E3", 0.25, BASS), (0.5, "G#3", 0.25, BASS),
])

# Bars 15–21: the full restatement — identical to bars 1–7, a touch softer.
restatement = phrase(3, [
    (0.0, "E5", 0.25, MELODY), (0.25, "D#5", 0.25, MELODY), (0.5, "E5", 0.25, MELODY),
    (0.75, "B4", 0.25, MELODY), (1.0, "D5", 0.25, MELODY), (1.25, "C5", 0.25, MELODY),
    (1.5, "A4", 0.5, 0.86), (2.25, "C4", 0.25, FILL), (2.5, "E4", 0.25, FILL), (2.75, "A4", 0.25, FILL),
    (3.0, "B4", 0.5, 0.86), (3.75, "E4", 0.25, FILL), (4.0, "G#4", 0.25, FILL), (4.25, "B4", 0.25, FILL),
    (4.5, "C5", 0.5, 0.86), (5.25, "E4", 0.25, FILL), (5.5, "E5", 0.25, MELODY), (5.75, "D#5", 0.25, MELODY),
    (6.0, "E5", 0.25, MELODY), (6.25, "D#5", 0.25, MELODY), (6.5, "E5", 0.25, MELODY),
    (6.75, "B4", 0.25, MELODY), (7.0, "D5", 0.25, MELODY), (7.25, "C5", 0.25, MELODY),
    (7.5, "A4", 0.5, 0.84), (8.25, "C4", 0.25, FILL), (8.5, "E4", 0.25, FILL), (8.75, "A4", 0.25, FILL),
    (9.0, "B4", 0.5, 0.84), (9.75, "E4", 0.25, FILL), (10.0, "C5", 0.25, 0.66), (10.25, "B4", 0.25, 0.6),
    (1.5, "A2", 0.25, BASS), (1.75, "E3", 0.25, BASS), (2.0, "A3", 0.25, BASS),
    (3.0, "E2", 0.25, BASS), (3.25, "E3", 0.25, BASS), (3.5, "G#3", 0.25, BASS),
    (4.5, "A2", 0.25, BASS), (4.75, "E3", 0.25, BASS), (5.0, "A3", 0.25, BASS),
    (7.5, "A2", 0.25, BASS), (7.75, "E3", 0.25, BASS), (8.0, "A3", 0.25, BASS),
    (9.0, "E2", 0.25, BASS), (9.25, "E3", 0.25, BASS), (9.5, "G#3", 0.25, BASS),
])

# Bar 22: home. The tonic, left to ring over one last broken chord.
finale = phrase(1, [
    (0.0, "A4", 1.5, 0.72),
    (0.0, "A2", 0.25, 0.46), (0.25, "E3", 0.25, 0.46), (0.5, "A3", 0.25, 0.46),
])

# A light, even hand on every phrase — baked in, deterministic.
for i, p in enumerate((theme_a, theme_b, episode, return_lead, restatement, finale)):
    song.arrange(piano, p.humanize(timing=0.12, velocity=0.05, seed=101 + i),
                 bars=[0, 5, 9, 14, 15, 22][i])

# The map: sections on true 3/8 bars, and a ritardando into the final tonic —
# hold tempo through bar 20, breathe through the cadence bar, land slowly.
song.add_section("theme", bar=0, bars=9)
song.add_section("episode", bar=9, bars=5)
song.add_section("return", bar=14, bars=8)
song.add_section("coda", bar=22, bars=1)
song.add_marker("high-answer", beat=19.0)
song.set_tempo_map([(0.0, BPM), (30.5, 64.0), (32.0, 54.0)])

# --- Compile, render, bounce --------------------------------------------------
program = song.compile(sample_rate=48_000)
print(f"compiled  hash={hex(program.hash)}")
print(f"          23 bars of 3/8 (plus pickup) at {BPM} bpm = "
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
with wave.open("fur_elise.wav", "wb") as w:
    w.setnchannels(2)
    w.setsampwidth(2)
    w.setframerate(program.sample_rate)
    w.writeframes(pcm.tobytes())
print("wrote     fur_elise.wav — press play.")
