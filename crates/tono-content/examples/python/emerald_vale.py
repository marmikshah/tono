"""emerald_vale — an original fantasy-village pastoral in G major (92 bpm,
6/8): a lilting concert-flute theme over nylon-guitar harp-rolls, a strings
pad that swells in at the second A, tiny glockenspiel answers in the B
section, and soft dotted-half-note bass roots.

The brief was "a pastoral RPG village theme" — village square at noon, not
battle. The melody is ORIGINAL, composed for this repo in that idiom: a
singable two-phrase arc, long holds across the bar lines, and grace notes
leaning into the downbeats.

Where fur_elise shows a 3/8 classical score and monsoon_melody the film-score
side, this one is built around the 6/8 meter map: arrangement and sections on
true 3-beat song-bars (phrase content in multiples of 3 beats), a one-bar
folk harp-roll per chord, and a rallentando over the final bar on the tempo
map — it loops sweetly back to the top.

Run with an installed tono wheel (or `maturin develop` in crates/tono-py):

    python3 crates/tono-content/examples/python/emerald_vale.py

It compiles the song, renders the mix, and writes `emerald_vale.wav`
(16-bit stereo) into the current directory — that's the track to listen to.
"""

import wave

import numpy as np
import tono

BPM = 92
BARS = 16  # song-bars of 6/8 — 3 beats each, 48 beats in all

song = tono.Song("emerald-vale", tempo=BPM, seed=4242)

# The piece is genuinely 6/8 — say so, and arrange()/sections take true
# song-bars of 3 beats. A Pattern still holds 4 beats per `bars=`, so a
# 4-song-bar phrase (12 beats of content) is a Pattern(bars=3).
song.set_meter_map([(0, 6, 8)])

# --- The band -------------------------------------------------------------
# The catalog voices stage hot (a lone nylon G2 can reach the master
# ceiling), so the faders sit low and the note gains do the shaping.
flute = song.track("flute", tono.instruments.flute("concert").gain(0.75).reverb(0.3).pan(0.06))
guitar = song.track("guitar", tono.instruments.guitar("nylon").gain(0.07).pan(-0.14).reverb(0.15).humanize(0.06))
strings = song.track("strings", tono.instruments.strings("ensemble").pan(-0.04).reverb(0.25))
bass = song.track("bass", tono.instruments.bass("finger").gain(0.6))
glock = song.track("glock", tono.instruments.mallets("glockenspiel").gain(0.6).pan(0.2).reverb(0.4))

# --- Harmony: one chord per song-bar, A A B A' -----------------------------
# G major. Song-bar 14 splits C | D across the bar's two halves — the
# predominant-to-dominant walk that sets up the final tonic.
CHORDS = [
    "G", "Em", "C", "D",    # a1
    "G", "Em", "C", "D",    # a2 — the strings swell in here
    "Am", "Bm", "C", "D",   # b — the relative-minor shade
    "G", "Em", ("C", "D"), "G",  # last-a, home on a held G
]
ROOTS = [
    "G1", "E1", "C2", "D2",
    "G1", "E1", "C2", "D2",
    "A1", "B1", "C2", "D2",
    "G1", "E1", ("C2", "D2"), "G1",
]

def tones(chord: str) -> list:
    """Four ascending chord tones from the symbol; triads get crowned with
    the octave root."""
    names = [p.name for p in tono.Chord(chord).pitches(3)]
    if len(names) == 3:
        names.append(tono.Pitch(names[0]).transpose(12).name)
    return names

# --- Guitar: a one-bar folk harp-roll per chord -----------------------------
def harp_roll(chord: str, gain: float = 0.44) -> tono.Pattern:
    """One 6/8 bar: the low root on beat 0, then the chord tones rolled
    upward in two 0.25-spaced ascents — one per half-bar pulse — left to
    ring into each other like a harp."""
    tri = [p.name for p in tono.Chord(chord).pitches(3)]
    up = [tono.Pitch(n).transpose(12).name for n in tri]
    low_root = tono.Pitch(tri[0]).transpose(-12).name
    low_fifth = tono.Pitch(tri[2]).transpose(-12).name
    p = tono.Pattern(bars=1)
    p.note(low_root, at=0.0, duration=2.9, gain=gain + 0.04)
    p.note(tri[0], at=0.50, duration=1.0, gain=gain - 0.16)
    p.note(tri[1], at=0.75, duration=1.0, gain=gain - 0.18)
    p.note(tri[2], at=1.00, duration=1.0, gain=gain - 0.18)
    p.note(low_fifth, at=1.50, duration=1.3, gain=gain - 0.14)
    p.note(up[0], at=2.00, duration=0.8, gain=gain - 0.20)
    p.note(up[1], at=2.25, duration=0.8, gain=gain - 0.22)
    p.note(up[2], at=2.50, duration=0.5, gain=gain - 0.22)
    return p

def harp_roll_two(a: str, b: str, gain: float = 0.5) -> tono.Pattern:
    """Two chords in one 6/8 bar — `a` on beats 0–1.5, `b` on beats 1.5–3."""
    p = tono.Pattern(bars=1)
    for at, chord in ((0.0, a), (1.5, b)):
        tri = [q.name for q in tono.Chord(chord).pitches(3)]
        low_root = tono.Pitch(tri[0]).transpose(-12).name
        p.note(low_root, at=at, duration=1.4, gain=gain + 0.04)
        p.note(tri[0], at=at + 0.50, duration=0.9, gain=gain - 0.16)
        p.note(tri[1], at=at + 0.75, duration=0.9, gain=gain - 0.18)
        p.note(tri[2], at=at + 1.00, duration=0.5, gain=gain - 0.18)
    return p

for bar, chord in enumerate(CHORDS):
    pat = harp_roll_two(*chord) if isinstance(chord, tuple) else harp_roll(chord)
    song.arrange(guitar, pat, bars=bar)

# --- Bass: soft dotted-half-note roots, one per song-bar --------------------
def bass_root(root: str, gain: float = 0.46) -> tono.Pattern:
    p = tono.Pattern(bars=1)
    p.note(root, at=0.0, duration=2.9, gain=gain)
    return p

def bass_two(r1: str, r2: str, gain: float = 0.5) -> tono.Pattern:
    """The split bar: one dotted-quarter root per half."""
    p = tono.Pattern(bars=1)
    p.note(r1, at=0.0, duration=1.4, gain=gain)
    p.note(r2, at=1.5, duration=1.4, gain=gain)
    return p

for bar, root in enumerate(ROOTS):
    pat = bass_two(*root) if isinstance(root, tuple) else bass_root(root)
    song.arrange(bass, pat, bars=bar)

# --- Strings: held pads from the second A on, swelled in --------------------
def pad(chord, gain: float = 0.5) -> tono.Pattern:
    """A whole-bar pad (two half-bar chords where the harmony splits)."""
    p = tono.Pattern(bars=1)
    if isinstance(chord, tuple):
        p.chord(tones(chord[0]), at=0.0, duration=1.5, gain=gain)
        p.chord(tones(chord[1]), at=1.5, duration=1.5, gain=gain)
    else:
        p.chord(tones(chord), at=0.0, duration=3.0, gain=gain)
    return p

for bar in range(4, BARS):
    song.arrange(strings, pad(CHORDS[bar]), bars=bar)
# The swell owns the strings' fader — no .gain() on that voice. In from
# nothing at the second A (bar 4 = beat 12), fullest through the B section,
# then let the last-a breathe back down.
song.automate(strings, "gain",
              [(12, 0.0), (16, 0.2), (24, 0.24), (36, 0.27), (44, 0.23), (48, 0.14)],
              curve="exp")

# --- The melody (original) ---------------------------------------------------
def phrase(bars: int, notes) -> tono.Pattern:
    p = tono.Pattern(bars=bars)
    for at, pitch, dur, gain in notes:
        p.note(pitch, at=at, duration=dur, gain=gain)
    return p

# Each phrase covers 4 song-bars of 6/8 = 12 beats = Pattern(bars=3).
# Theme A (song-bars 0–3): the question — down from D5 to the low tonic,
# back up through E5, crowned with a held G5 before settling on D5.
theme_a = phrase(3, [
    (0.00, "D5", 1.00, 0.80), (1.00, "B4", 0.50, 0.74), (1.50, "G4", 1.00, 0.76),
    (2.50, "A4", 0.50, 0.72),
    (3.00, "B4", 1.00, 0.80), (4.00, "E5", 0.50, 0.84), (4.50, "D5", 1.00, 0.82),
    (5.50, "B4", 0.25, 0.68), (5.75, "D5", 0.25, 0.70),   # twin graces into C5
    (6.00, "C5", 1.00, 0.82), (7.00, "E5", 0.50, 0.78), (7.50, "G5", 1.50, 0.88),
    (9.00, "F#5", 0.50, 0.80), (9.50, "E5", 0.50, 0.76), (10.00, "D5", 2.00, 0.84),
])

# Theme A2 (song-bars 4–7): the same head, a higher arc — the A5 held clear
# across the bar line into the D bar, then graces tumbling into the B strain.
theme_a2 = phrase(3, [
    (0.00, "D5", 1.00, 0.80), (1.00, "B4", 0.50, 0.74), (1.50, "G4", 1.00, 0.76),
    (2.50, "A4", 0.50, 0.72),
    (3.00, "B4", 1.00, 0.80), (4.00, "E5", 0.50, 0.84), (4.50, "G5", 1.00, 0.88),
    (5.50, "F#5", 0.50, 0.82),
    (6.00, "E5", 1.00, 0.84), (7.00, "G5", 0.50, 0.88), (7.50, "A5", 2.00, 0.94),
    (9.50, "G5", 0.50, 0.80), (10.00, "F#5", 0.50, 0.78), (10.50, "E5", 1.00, 0.80),
    (11.50, "D5", 0.25, 0.70), (11.75, "E5", 0.25, 0.72),
])

# The B strain (song-bars 8–11): the relative-minor shade — a dip to low A,
# a climb to the piece's second A5 peak, and a long D5 the glock answers.
bridge = phrase(3, [
    (0.00, "E5", 1.50, 0.86), (1.50, "C5", 0.50, 0.76), (2.00, "A4", 0.50, 0.74),
    (3.00, "B4", 0.50, 0.78), (3.50, "D5", 0.50, 0.80), (4.00, "F#5", 1.50, 0.86),
    (6.00, "G5", 1.00, 0.88), (7.00, "A5", 0.50, 0.90), (7.50, "G5", 1.00, 0.86),
    (8.50, "E5", 0.50, 0.78),
    (9.00, "D5", 2.00, 0.86),
    (11.00, "C5", 0.25, 0.72), (11.25, "B4", 0.25, 0.72), (11.50, "A4", 0.50, 0.74),
])

# Last A (song-bars 12–15): the head one more time, then home — grace notes
# into the final bar and the tonic left ringing through the rallentando.
last_a = phrase(3, [
    (0.00, "D5", 1.00, 0.80), (1.00, "B4", 0.50, 0.74), (1.50, "G4", 1.00, 0.76),
    (2.50, "A4", 0.50, 0.72),
    (3.00, "B4", 1.00, 0.80), (4.00, "E5", 0.50, 0.84), (4.50, "D5", 1.00, 0.82),
    (5.50, "B4", 0.50, 0.76),
    (6.00, "C5", 1.00, 0.80), (7.00, "E5", 0.50, 0.78),   # over C …
    (7.50, "D5", 1.00, 0.80), (8.50, "C5", 0.25, 0.70), (8.75, "B4", 0.25, 0.72),  # … over D
    (9.00, "B4", 0.75, 0.82), (9.75, "A4", 0.25, 0.70), (10.00, "G4", 2.00, 0.84),
])

# A light, even hand on the flute — baked in, deterministic.
phrases = [theme_a, theme_a2, bridge, last_a]
phrases = [p.humanize(timing=0.12, velocity=0.05, seed=501 + i) for i, p in enumerate(phrases)]
for p, bar in zip(phrases, (0, 4, 8, 12)):
    song.arrange(flute, p, bars=bar)

# --- Glockenspiel: tiny answers in the B section only -----------------------
# Each chime rhymes with the flute phrase it follows — an octave-and-more up.
glock_answers = phrase(3, [
    (2.50, "A5", 0.50, 0.30),
    (5.50, "D6", 0.50, 0.28),
    (10.50, "F#6", 0.50, 0.26),
])
song.arrange(glock, glock_answers.humanize(timing=0.08, velocity=0.04, seed=505), bars=8)

# --- Rides and the map -------------------------------------------------------
song.add_section("a1", bar=0, bars=4)
song.add_section("a2", bar=4, bars=4)
song.add_section("b", bar=8, bars=4)
song.add_section("last-a", bar=12, bars=4)
song.add_marker("strings-swell", beat=12)
song.add_marker("reprise", beat=36)
song.add_marker("rallentando", beat=45)
# Hold 92 through song-bar 14, then breathe across the final bar (beats
# 45–48) and let the held G carry the loop back around.
song.set_tempo_map([(0.0, float(BPM)), (45.0, 84.0), (46.5, 72.0), (47.5, 62.0)])

# --- Compile, render, bounce --------------------------------------------------
program = song.compile(sample_rate=48_000)
print(f"compiled  hash={hex(program.hash)}")
print(f"          {BARS} bars of 6/8 at {BPM} bpm = "
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
with wave.open("emerald_vale.wav", "wb") as w:
    w.setnchannels(2)
    w.setsampwidth(2)
    w.setframerate(program.sample_rate)
    w.writeframes(pcm.tobytes())
print("wrote     emerald_vale.wav — press play.")
