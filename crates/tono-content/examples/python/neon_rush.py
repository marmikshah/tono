"""neon_rush — an original synthwave/outrun racing piece in C minor (104 bpm):
a four-on-the-floor tr808 under a 16th-note octave bass, dx stabs on the
offbeats, and a rock-organ lead standing in for the synth lead. Night
highway, neon, momentum.

The melody is ORIGINAL, written for this repo: an 8th-note C-minor-
pentatonic riff with octave jumps, a hook up the octave crowned by a high
held Eb that resolves 4–3 over the Bb bar, and a return that drops the riff
a fifth into F-pentatonic shade before the wind-down lands on a clean Cm —
the ending texture echoes the intro, so the piece loops decently.

On the feature side: a euclidean tresillo clap layer, a seeded probability
hat-16ths shimmer, swing shared by drums and bass, per-phrase humanize with
fixed seeds, one gain automation ride on the stabs, and named sections with
a "hook" marker.

Run with an installed tono wheel (or `maturin develop` in crates/tono-py):

    python3 crates/tono-content/examples/python/neon_rush.py

It compiles the song, renders the mix, and writes `neon_rush.wav` (16-bit
stereo) into the current directory — that's the track to listen to.
"""

import wave

import numpy as np
import tono

BPM = 104
BARS = 16

song = tono.Song("neon-rush", tempo=BPM, seed=1984)

# --- The band -------------------------------------------------------------
drums = song.track("drums", tono.instruments.drums("tr808").gain(0.45).swing(0.1).humanize(0.08))
bass = song.track("bass", tono.instruments.bass("synth").gain(0.5).swing(0.1))
keys = song.track("keys", tono.instruments.electric_piano("dx").pan(-0.12).reverb(0.18))
lead = song.track("lead", tono.instruments.organ("rock").gain(0.5).reverb(0.2).pan(0.06))
glock = song.track("glock", tono.instruments.mallets("glockenspiel").gain(0.4).reverb(0.35).pan(0.18))

# --- Harmony: Cm – Ab – Eb – Bb (i–VI–III–VII), one chord per bar ---------
# Four passes around the loop; the last swaps the Eb for a second Bb so bars
# 14–15 cadence VII → i and land clean on the tonic.
CHORDS = [
    "Cm", "Ab", "Eb", "Bb",
    "Cm", "Ab", "Eb", "Bb",
    "Cm", "Ab", "Eb", "Bb",
    "Cm", "Ab", "Bb", "Cm",
]

def tones(chord: str) -> list:
    """Four ascending chord tones from the symbol; triads get crowned with
    the octave root."""
    names = [p.name for p in tono.Chord(chord).pitches(4)]
    if len(names) == 3:
        names.append(tono.Pitch(names[0]).transpose(12).name)
    return names

# --- Bass: the 16th-note octave engine, root–root–octave–root per beat ----
ROOTS = {"Cm": "C2", "Ab": "Ab1", "Eb": "Eb2", "Bb": "Bb1"}

def bass_bar(root: str, gain: float = 0.9) -> tono.Pattern:
    octave = tono.Pitch(root).transpose(12).name
    p = tono.Pattern(bars=1)
    for beat in range(4):
        g = gain if beat == 0 else gain * 0.92
        p.note(root, at=beat + 0.00, duration=0.22, gain=g)
        p.note(root, at=beat + 0.25, duration=0.22, gain=g * 0.6)
        p.note(octave, at=beat + 0.50, duration=0.22, gain=g * 0.8)
        p.note(root, at=beat + 0.75, duration=0.22, gain=g * 0.65)
    return p

for bar, chord in enumerate(CHORDS[:15]):
    song.arrange(bass, bass_bar(ROOTS[chord]), bars=bar)
# The landing bar keeps the engine idling at half level — it echoes bar 0,
# so the loop seam is just the intro again.
song.arrange(bass, bass_bar("C2").vel(0.7), bars=15)

# --- Drums: four-on-the-floor, snare on 2 & 4, offbeat openhats -----------
kick4 = tono.Pattern(bars=1)
kick4.hit("kick", beats=[0, 1, 2, 3])

hats16 = tono.Pattern(bars=1)
hats16.hit("hat", beats=[x * 0.25 for x in range(16)])
# A seeded keep/drop turns the 16ths into a fixed shimmer — same every run.
hats = hats16.probability(0.65, seed=811).vel(0.45)

# The euclidean element: a tresillo clap (3 over 8), mirrored into the bar's
# second half for the classic double-tresillo drive.
tres = tono.Pattern.euclidean(3, 8, "midi:39")
claps = tres.layer(tres.rotate(8)).vel(0.7)

groove = tono.Pattern(bars=1)
groove.hit("kick", beats=[0, 1, 2, 3])
groove.hit("snare", beats=[1, 3])
groove.hit("openhat", beats=[x + 0.5 for x in range(4)])
full = groove.layer(claps).layer(hats)

fill = tono.Pattern(bars=1)
fill.hit("kick", beats=[0, 1, 2])
fill.hit("snare", beats=[1, 3, 3.25, 3.5, 3.75])
fill.hit("openhat", beats=[0.5, 1.5])

crash = tono.Pattern(bars=1)
crash.hit("crash", beats=[0])

song.arrange(drums, kick4, bars=0)              # intro: the engine alone
song.arrange(drums, kick4.layer(hats), bars=1)  # hats sneak in with the stabs
song.arrange(drums, full, bars=[2, 3, 4, 6, 7, 8, 10, 11, 12])
song.arrange(drums, fill, bars=[5, 9, 13])      # a snare rush every fourth bar
song.arrange(drums, crash, bars=[2, 6, 10])     # section downbeats
song.arrange(drums, groove.vel(0.85), bars=14)  # wind down
song.arrange(drums, kick4.vel(0.7), bars=15)    # the loop seam

# --- Keys: dx stabs on the offbeats, short and glassy ----------------------
def stabs(chord: str, gain: float = 0.5) -> tono.Pattern:
    p = tono.Pattern(bars=1)
    for i, at in enumerate((0.5, 1.5, 2.5, 3.5)):
        p.chord(tones(chord), at=at, duration=0.3, gain=gain * (1.0 if i % 2 == 0 else 0.85))
    return p

for bar in range(1, 15):
    song.arrange(keys, stabs(CHORDS[bar]), bars=bar)
last = tono.Pattern(bars=1)
last.chord(tones("Cm"), at=0.5, duration=0.3, gain=0.4)
song.arrange(keys, last, bars=15)
# The one gain ride: the stabs bloom into the hook and dim for the landing.
# The lane owns the fader, so the dx voice itself sets no gain.
song.automate(keys, "gain", [(4, 0.55), (8, 0.64), (24, 0.72), (40, 0.66), (56, 0.46), (63, 0.36)])

# --- The lead (original) ----------------------------------------------------
def phrase(bars: int, notes) -> tono.Pattern:
    p = tono.Pattern(bars=bars)
    for at, pitch, dur, gain in notes:
        p.note(pitch, at=at, duration=dur, gain=gain)
    return p

# Riff A (bars 2–5): an 8th-note C-minor-pentatonic drive — two even hits,
# then the octave jump; each bar answers the jump with a short descent.
riff_a = phrase(4, [
    (0.00, "C5", 0.45, 0.78), (0.50, "C5", 0.45, 0.70), (1.00, "C6", 0.45, 0.84),
    (1.50, "C5", 0.45, 0.72), (2.00, "Eb5", 0.45, 0.78), (2.50, "G5", 0.45, 0.80),
    (3.00, "Eb5", 0.45, 0.74), (3.50, "C5", 0.45, 0.70),
    (4.00, "Eb5", 0.45, 0.78), (4.50, "Eb5", 0.45, 0.70), (5.00, "Eb6", 0.45, 0.84),
    (5.50, "Eb5", 0.45, 0.72), (6.00, "F5", 0.45, 0.78), (6.50, "Eb5", 0.45, 0.74),
    (7.00, "C5", 0.45, 0.76), (7.50, "Bb4", 0.45, 0.70),
    (8.00, "G5", 0.45, 0.80), (8.50, "G5", 0.45, 0.72), (9.00, "G6", 0.45, 0.86),
    (9.50, "G5", 0.45, 0.74), (10.00, "Bb5", 0.45, 0.80), (10.50, "G5", 0.45, 0.76),
    (11.00, "F5", 0.45, 0.74), (11.50, "Eb5", 0.45, 0.72),
    (12.00, "F5", 0.45, 0.78), (12.50, "F5", 0.45, 0.70), (13.00, "F6", 0.45, 0.85),
    (13.50, "F5", 0.45, 0.72), (14.00, "Bb5", 0.45, 0.80), (14.50, "F5", 0.45, 0.74),
    (15.00, "Eb5", 0.45, 0.74), (15.50, "C5", 0.45, 0.72),
])

# The hook (bars 6–9): the same drive up an octave, crowned by a high Eb
# held across the Bb bar — a 4–3 suspension that resolves to D at beat 3.
hook = phrase(4, [
    (0.00, "C6", 0.45, 0.82), (0.50, "C6", 0.45, 0.74), (1.00, "Eb6", 0.45, 0.86),
    (1.50, "G6", 0.45, 0.88), (2.00, "F6", 0.45, 0.82), (2.50, "Eb6", 0.45, 0.78),
    (3.00, "F6", 0.45, 0.82), (3.50, "G6", 0.45, 0.86),
    (4.00, "Eb6", 2.50, 0.92),
    (7.00, "D6", 0.50, 0.84), (7.50, "Bb5", 0.45, 0.80),
    (8.00, "C6", 0.45, 0.86), (8.50, "C6", 0.45, 0.76), (9.00, "C7", 0.45, 0.90),
    (9.50, "C6", 0.45, 0.78), (10.00, "Eb6", 0.45, 0.82), (10.50, "C6", 0.45, 0.78),
    (11.00, "G5", 0.45, 0.74), (11.50, "Bb5", 0.45, 0.78),
    (12.00, "Eb6", 0.45, 0.86), (12.50, "C6", 0.45, 0.78), (13.00, "Bb5", 0.45, 0.78),
    (13.50, "C6", 0.45, 0.80), (14.00, "G5", 1.50, 0.84),
])

# The return (bars 10–13): riff A transposed a fifth down — F-pentatonic
# shade over the same changes, cooler after the hook's glare.
variant = riff_a.transpose(-7)

# Wind down (bars 14–15): the riff's shape walked downhill, then a held high
# C over the tonic — a clean landing with an open seam for the loop.
wind = phrase(2, [
    (0.00, "C6", 0.45, 0.80), (0.50, "Bb5", 0.45, 0.74), (1.00, "G5", 0.45, 0.76),
    (1.50, "F5", 0.45, 0.72), (2.00, "Eb5", 0.45, 0.76), (2.50, "C5", 0.45, 0.72),
    (3.00, "Bb4", 0.45, 0.70), (3.50, "C5", 0.45, 0.74),
    (4.00, "C6", 3.00, 0.80),
])

# A light, even hand on the lead — baked in, deterministic, one fixed seed
# per phrase.
phrases = [riff_a, hook, variant, wind]
phrases = [p.humanize(timing=0.08, velocity=0.05, seed=401 + i) for i, p in enumerate(phrases)]
song.arrange(lead, phrases[0], bars=2)
song.arrange(lead, phrases[1], bars=6)
song.arrange(lead, phrases[2].vel(0.95), bars=10)
song.arrange(lead, phrases[3], bars=14)

# --- Glockenspiel: doubling the hook's peaks at low velocity -----------------
peaks = tono.Pattern(bars=4)
peaks.note("Eb7", at=4.0, duration=2.5, gain=0.30)   # the held suspension
peaks.note("D7", at=7.0, duration=0.5, gain=0.28)    # its resolution
peaks.note("C7", at=8.0, duration=0.45, gain=0.32)   # the Cm crown
peaks.note("C7", at=9.0, duration=0.45, gain=0.30)   # ... and its octave jump
peaks.note("G6", at=14.0, duration=1.5, gain=0.26)   # the cadence note
song.arrange(glock, peaks, bars=6)

# --- Sections and the marker --------------------------------------------------
song.add_section("intro", bar=0, bars=2)
song.add_section("groove", bar=2, bars=4)
song.add_section("hook", bar=6, bars=4)
song.add_section("return", bar=10, bars=4)
song.add_section("outro", bar=14, bars=2)
song.add_marker("hook", beat=24)

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
with wave.open("neon_rush.wav", "wb") as w:
    w.setnchannels(2)
    w.setsampwidth(2)
    w.setframerate(program.sample_rate)
    w.writeframes(pcm.tobytes())
print("wrote     neon_rush.wav — press play.")
