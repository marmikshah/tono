"""monsoon_melody — an original Bollywood-style romantic ballad in D minor
(~86 bpm): a bansuri-like flute lead over nylon-guitar arpeggios, a strings
pad, and a laid-back half-time groove.

The brief was "a popular Bollywood song, as an instrumental" — but film-song
melodies are copyrighted compositions, so a transcription can't ship in the
repo. This is an ORIGINAL melody written in that idiom instead: minor-key
yearning, turns that approach the beat from above, and the high wail in the
lift over i–VI–III–VII and i–VI–iv–V.

Where fur_elise shows the score-faithful side and golden_hour the producer
side, this one shows the film-score side: chord-symbol voicings straight from
`tono.Chord`, half-bar chord changes, a swung acoustic kit in half time, a
glockenspiel shadowing the lift an octave up, and a ritardando outro.

Run with an installed tono wheel (or `maturin develop` in crates/tono-py):

    python3 crates/tono-content/examples/python/monsoon_melody.py

It compiles the song, renders the mix, and writes `monsoon_melody.wav`
(16-bit stereo) into the current directory — that's the track to listen to.
"""

import wave

import numpy as np
import tono

BPM = 86
BARS = 18

song = tono.Song("monsoon-melody", tempo=BPM, seed=2026)

# --- The band -------------------------------------------------------------
flute = song.track("flute", tono.instruments.flute("concert").gain(1.15).reverb(0.3).pan(0.06))
guitar = song.track("guitar", tono.instruments.guitar("nylon").pan(-0.14).reverb(0.15).humanize(0.08))
strings = song.track("strings", tono.instruments.strings("ensemble").pan(-0.04).reverb(0.2))
bass = song.track("bass", tono.instruments.bass("finger").gain(0.85).swing(0.14))
drums = song.track("drums", tono.instruments.drums("acoustic").swing(0.14).humanize(0.12))
glock = song.track("glock", tono.instruments.mallets("glockenspiel").pan(0.2).reverb(0.4))

# --- Harmony: one chord per bar, with a dominant lift halfway up ----------
# D minor. The (Gm, A7) bars change on beat 2 — the V that pulls each phrase
# home, and where the melody's one C# lives.
CHORDS = [
    "Dm", "Bb",             # intro (guitar alone)
    "Dm", "Bb", "Gm", ("Gm", "A7"),  # theme
    "Dm", "Bb", "Gm", ("Gm", "A7"),  # reprise
    "F", "C", "Bb", ("Gm", "A7"),    # the lift
    "Gm", ("Gm", "A7"),     # recall
    "Dm", "Dm",             # outro
]

def tones(chord: str) -> list:
    """Four ascending chord tones from the symbol; triads get crowned with
    the octave root."""
    names = [p.name for p in tono.Chord(chord).pitches(4)]
    if len(names) == 3:
        names.append(tono.Pitch(names[0]).transpose(12).name)
    return names

# --- Guitar: pima-style arpeggio, low root on the downbeat ----------------
def arpeggio(chord: str, gain: float = 0.55) -> tono.Pattern:
    names = tones(chord)
    root = tono.Pitch(names[0]).transpose(-12).name
    p = tono.Pattern(bars=1)
    p.note(root, at=0.0, duration=0.9, gain=gain + 0.12)
    for i, idx in enumerate((1, 2, 3, 2, 1, 2, 3)):
        p.note(names[idx], at=0.5 * (i + 1), duration=0.45, gain=gain)
    return p

def arpeggio_two(a: str, b: str, gain: float = 0.55) -> tono.Pattern:
    """Two chords in one bar — `a` on beats 0–1, `b` on beats 2–3."""
    p = tono.Pattern(bars=1)
    for at, chord in ((0.0, a), (2.0, b)):
        names = tones(chord)
        root = tono.Pitch(names[0]).transpose(-12).name
        p.note(root, at=at, duration=0.9, gain=gain + 0.12)
        for i, idx in enumerate((1, 2, 3)):
            p.note(names[idx], at=at + 0.5 * (i + 1), duration=0.45, gain=gain)
    return p

for bar, chord in enumerate(CHORDS):
    pat = arpeggio_two(*chord) if isinstance(chord, tuple) else arpeggio(chord)
    song.arrange(guitar, pat, bars=bar)

# --- Bass: in with the band at bar 2; root-fifth, locked to the kick ------
ROOTS = {2: "D2", 3: "Bb1", 4: "G1", 6: "D2", 7: "Bb1", 8: "G1",
         10: "F2", 11: "C2", 12: "Bb1", 14: "G1"}
TWO = {5: ("G1", "A1"), 9: ("G1", "A1"), 13: ("G1", "A1"), 15: ("G1", "A1")}

def bass_bar(root: str, gain: float = 0.85) -> tono.Pattern:
    fifth = tono.Pitch(root).transpose(7).name
    p = tono.Pattern(bars=1)
    p.note(root, at=0.0, duration=1.5, gain=gain)
    p.note(root, at=2.0, duration=0.5, gain=gain * 0.9)
    p.note(fifth, at=2.75, duration=0.25, gain=gain * 0.8)
    p.note(root, at=3.25, duration=0.75, gain=gain * 0.9)
    return p

def bass_two(r1: str, r2: str, gain: float = 0.85) -> tono.Pattern:
    fifth2 = tono.Pitch(r2).transpose(7).name
    p = tono.Pattern(bars=1)
    p.note(r1, at=0.0, duration=1.75, gain=gain)
    p.note(r2, at=2.0, duration=1.25, gain=gain)
    p.note(fifth2, at=3.5, duration=0.5, gain=gain * 0.85)
    return p

def bass_whole(root: str, gain: float = 0.6) -> tono.Pattern:
    p = tono.Pattern(bars=1)
    p.note(root, at=0.0, duration=3.5, gain=gain)
    return p

for bar, root in ROOTS.items():
    song.arrange(bass, bass_bar(root), bars=bar)
for bar, (r1, r2) in TWO.items():
    song.arrange(bass, bass_two(r1, r2), bars=bar)
song.arrange(bass, bass_whole("D2"), bars=16)
song.arrange(bass, bass_whole("D2").vel(0.6), bars=17)

# --- Drums: half-time ballad groove, swung --------------------------------
soft = tono.Pattern(bars=1)
soft.hit("kick", beats=[0, 2.5])
soft.hit("snare", beats=[2])
soft.hit("hat", beats=[x * 0.5 for x in range(8)])

lift_groove = tono.Pattern(bars=1)
lift_groove.hit("kick", beats=[0, 1.75, 2.5])
lift_groove.hit("snare", beats=[2])
lift_groove.hit("hat", beats=[x * 0.5 for x in range(8)])
lift_groove.hit("openhat", beats=[3.5])

fill = tono.Pattern(bars=1)
fill.hit("kick", beats=[0, 2.5])
fill.hit("snare", beats=[2, 3.25, 3.5, 3.75])

crash = tono.Pattern(bars=1)
crash.hit("kick", beats=[0])
crash.hit("crash", beats=[0])

song.arrange(drums, soft, bars=list(range(2, 9)))
song.arrange(drums, fill, bars=9)           # wind up into the lift
song.arrange(drums, crash, bars=10)
song.arrange(drums, lift_groove, bars=[10, 11, 12])
song.arrange(drums, fill.vel(0.7), bars=13)  # downshift into the recall
song.arrange(drums, soft, bars=[14, 15])
song.automate(drums, "gain", [(8, 0.75), (40, 1.0), (56, 0.7)])

# --- Strings: held pads from the theme on, swelling into the lift ---------
def pad(chord, gain: float = 0.5) -> tono.Pattern:
    p = tono.Pattern(bars=1)
    if isinstance(chord, tuple):
        p.chord(tones(chord[0]), at=0, duration=2, gain=gain)
        p.chord(tones(chord[1]), at=2, duration=2, gain=gain)
    else:
        p.chord(tones(chord), at=0, duration=4, gain=gain)
    return p

for bar in range(2, BARS):
    song.arrange(strings, pad(CHORDS[bar]), bars=bar)
song.automate(strings, "gain",
              [(8, 0.0), (12, 0.5), (36, 0.55), (40, 0.8), (52, 0.8), (56, 0.5), (64, 0.35)],
              curve="exp")

# --- The melody (original) -------------------------------------------------
def phrase(bars: int, notes) -> tono.Pattern:
    p = tono.Pattern(bars=bars)
    for at, pitch, dur, gain in notes:
        p.note(pitch, at=at, duration=dur, gain=gain)
    return p

# Bar 1's pickup: the flute answers the guitar's intro — a breath, then in.
pickup = phrase(1, [
    (3.0, "A4", 0.5, 0.66), (3.5, "C5", 0.5, 0.70),
])

# Theme A (bars 2–3): the question — rise to F, fall back, then climb to A.
theme_a = phrase(2, [
    (0.00, "D5", 0.50, 0.78), (0.50, "E5", 0.25, 0.72), (0.75, "F5", 1.00, 0.86),
    (1.75, "E5", 0.25, 0.74), (2.00, "D5", 0.50, 0.78), (2.50, "C5", 0.50, 0.72),
    (3.00, "D5", 1.00, 0.84),
    (4.00, "F5", 0.50, 0.80), (4.50, "G5", 0.50, 0.84), (5.00, "A5", 0.75, 0.90),
    (5.75, "G5", 0.25, 0.76), (6.00, "F5", 1.50, 0.86), (7.50, "E5", 0.50, 0.74),
])

# Theme B (bars 4–5): the answer — a long sigh back down to the low tonic.
theme_b = phrase(2, [
    (0.00, "C5", 0.50, 0.76), (0.50, "D5", 0.25, 0.72), (0.75, "E5", 0.75, 0.82),
    (1.50, "C5", 0.25, 0.70), (2.00, "A4", 1.00, 0.80), (3.00, "G4", 0.50, 0.70),
    (3.50, "A4", 0.50, 0.72),
    (4.00, "Bb4", 0.50, 0.74), (4.50, "A4", 0.25, 0.68), (4.75, "G4", 0.25, 0.66),
    (5.00, "F4", 0.75, 0.72), (5.75, "E4", 0.25, 0.64), (6.00, "D4", 2.00, 0.80),
])

# The lift (bars 10–13): the wail — a leap to high A, the Bb crown over the
# VI, and the one C# in the piece leaning on the dominant before home.
lift = phrase(4, [
    (0.00, "A5", 0.75, 0.94), (0.75, "G5", 0.25, 0.80), (1.00, "F5", 0.50, 0.86),
    (1.50, "E5", 0.50, 0.82), (2.00, "F5", 1.00, 0.90), (3.00, "E5", 0.50, 0.80),
    (3.50, "D5", 0.50, 0.78),
    (4.00, "E5", 0.50, 0.82), (4.50, "F5", 0.25, 0.76), (4.75, "E5", 0.25, 0.74),
    (5.00, "D5", 1.50, 0.86), (6.50, "E5", 0.50, 0.78), (7.00, "F5", 0.50, 0.82),
    (7.50, "G5", 0.50, 0.86),
    (8.00, "A5", 1.50, 0.97), (9.50, "Bb5", 0.50, 0.90), (10.00, "A5", 0.50, 0.92),
    (10.50, "G5", 0.50, 0.86), (11.00, "F5", 1.00, 0.88),
    (12.00, "G5", 0.50, 0.86), (12.50, "F5", 0.25, 0.78), (12.75, "E5", 0.25, 0.76),
    (13.00, "D5", 1.75, 0.90), (14.75, "C#5", 0.25, 0.72), (15.00, "D5", 0.75, 0.84),
])

# Outro (bars 16–17): one last F–E–D, and the tonic left to ring through the
# ritardando.
outro = phrase(2, [
    (0.00, "F5", 0.75, 0.82), (0.75, "E5", 0.25, 0.72), (1.00, "D5", 5.50, 0.80),
])

# A light, even hand on the flute — baked in, deterministic. The lift's
# humanized pattern is reused for the glockenspiel shadow so the two agree.
phrases = [pickup, theme_a, theme_b, theme_a, theme_b, lift, theme_b, outro]
phrases = [p.humanize(timing=0.12, velocity=0.05, seed=301 + i) for i, p in enumerate(phrases)]
for p, bar in zip(phrases, (1, 2, 4, 6, 8, 10, 14, 16)):
    song.arrange(flute, p if bar != 14 else p.vel(0.85), bars=bar)

# The glockenspiel shadows the lift an octave up — just sparkle, not a lead.
song.arrange(glock, phrases[5].transpose(12).vel(0.35), bars=10)

# --- Rides and the map -----------------------------------------------------
song.automate(guitar, "gain", [(0, 0.85), (40, 0.7), (64, 0.9)])
song.add_section("intro", bar=0, bars=2)
song.add_section("theme", bar=2, bars=4)
song.add_section("reprise", bar=6, bars=4)
song.add_section("lift", bar=10, bars=4)
song.add_section("recall", bar=14, bars=2)
song.add_section("outro", bar=16, bars=2)
song.add_marker("flute-enters", beat=8)
song.add_marker("lift", beat=40)
# Hold tempo through the recall, then breathe through the last two bars.
song.set_tempo_map([(0.0, float(BPM)), (64.0, 78.0), (66.0, 68.0), (68.0, 56.0)])

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
with wave.open("monsoon_melody.wav", "wb") as w:
    w.setnchannels(2)
    w.setsampwidth(2)
    w.setframerate(program.sample_rate)
    w.writeframes(pcm.tobytes())
print("wrote     monsoon_melody.wav — press play.")
