"""puzzle_menu — an ORIGINAL quirky puzzle/menu theme in C major (112 bpm):
a staccato marimba lead over a bouncy root–5th bass, vibraphone answers in
the gaps, glockenspiel sprinkles at the phrase ends, and a feather-light
acoustic kit. Bright, bouncy, a little cheeky — and staccato everywhere.

The tune is ONE two-bar motif plus pattern transforms — that's the point of
the piece: the A' section is the motif rotated a bar, the B section is the
motif mirrored with `reverse`, and the last A'' is the motif transposed down
a fifth, which turns the motif's cheeky F–F#–G chromatic approach into
A#–B–C, a leading-tone landing on a clean C. `Pattern.gate` does the pluck.

Where monsoon_melody shows the film-score side and golden_hour the producer
side, this one shows the puzzle-game side: a loop-sized 16-bar form
(a / a-prime / b / last-a) built from `tono.Chord` progressions, composed
entirely by transform derivation.

Run with an installed tono wheel (or `maturin develop` in crates/tono-py):

    python3 crates/tono-content/examples/python/puzzle_menu.py

It compiles the song, renders the mix, and writes `puzzle_menu.wav`
(16-bit stereo) into the current directory — that's the track to listen to.
"""

import wave

import numpy as np
import tono

BPM = 112
BARS = 16

song = tono.Song("puzzle-menu", tempo=BPM, seed=112)

# --- The band -------------------------------------------------------------
marimba = song.track("marimba", tono.instruments.mallets("marimba").pan(-0.08).reverb(0.15))
vibes = song.track("vibes", tono.instruments.mallets("vibraphone").gain(0.8).pan(0.16).reverb(0.3))
bass = song.track("bass", tono.instruments.bass("finger").gain(0.78))
drums = song.track("drums", tono.instruments.drums("acoustic").gain(0.72).humanize(0.10))
glock = song.track("glock", tono.instruments.mallets("glockenspiel").gain(0.7).pan(0.28).reverb(0.4))

# --- Harmony: one chord per bar -------------------------------------------
# C major. A and the last A'' share I–IV–V–I; A' dips to the relative minor;
# B is the IV–V–vi–V turnaround. The final C bar lands clean, so the loop
# back to bar 0 is seamless.
CHORDS = [
    "C", "F", "G", "C",     # a
    "Am", "F", "G", "C",    # a-prime
    "F", "G", "Am", "G",    # b
    "C", "F", "G", "C",     # last-a
]
ROOTS = {"C": "C2", "F": "F2", "G": "G2", "Am": "A2"}

def tones(chord: str) -> list:
    """Four ascending chord tones from the symbol; triads get crowned with
    the octave root."""
    names = [p.name for p in tono.Chord(chord).pitches(4)]
    if len(names) == 3:
        names.append(tono.Pitch(names[0]).transpose(12).name)
    return names

# --- Bass: bouncy root–5th 8ths, gated to a pluck --------------------------
def bass_bar(root: str, gain: float = 0.82) -> tono.Pattern:
    fifth = tono.Pitch(root).transpose(7).name
    p = tono.Pattern(bars=1)
    for i in range(4):
        p.note(root, at=1.0 * i, duration=0.5, gain=gain)
        p.note(fifth, at=0.5 + 1.0 * i, duration=0.5, gain=gain * 0.78)
    return p

for bar, chord in enumerate(CHORDS):
    song.arrange(bass, bass_bar(ROOTS[chord]).gate(0.6), bars=bar)

# --- Drums: light kit — kick [0, 2], snare [1, 3] half-cocked, hat 8ths -----
kick = tono.Pattern(bars=1)
kick.hit("kick", beats=[0, 2])
kick = kick.vel(0.8)   # light kit: the kick stays a heartbeat, not a thump
snare = tono.Pattern(bars=1)
snare.hit("snare", beats=[1, 3])
hat = tono.Pattern(bars=1)
hat.hit("hat", beats=[x * 0.5 for x in range(8)])
groove = kick.layer(snare.vel(0.5)).layer(hat.vel(0.55))

# A tiny 16th-note snare fill into the B section, echoed softer into last-a.
fill_snare = tono.Pattern(bars=1)
fill_snare.hit("snare", beats=[1, 3, 3.5, 3.75])
fill = kick.layer(fill_snare.vel(0.55)).layer(hat.vel(0.55))

# A soft crash crowns the B and final-A entrances.
crash = tono.Pattern(bars=1)
crash.hit("crash", beats=[0])

for bar in range(BARS):
    if bar == 7:
        song.arrange(drums, fill, bars=bar)
    elif bar == 11:
        song.arrange(drums, fill.vel(0.8), bars=bar)
    elif bar == 8:
        song.arrange(drums, groove.layer(crash.vel(0.45)), bars=bar)
    elif bar == 12:
        song.arrange(drums, groove.layer(crash.vel(0.35)), bars=bar)
    else:
        song.arrange(drums, groove, bars=bar)

# --- Vibraphone: two-note harmony answers in the motif's gaps --------------
# The motif leaves a one-beat gap each phrase (beats 3–4 of its first bar);
# the reversed motif's gap opens its second bar instead. The vibes answer
# there — the 5th up to the octave root ("up") or back down to the 3rd.
def vibes_answer(chord: str, kind: str, at: float = 3.0, gain: float = 0.46) -> tono.Pattern:
    t = tones(chord)
    pair = (t[2], t[3]) if kind == "up" else (t[2], t[1])
    p = tono.Pattern(bars=1)
    p.note(pair[0], at=at, duration=0.4, gain=gain)
    p.note(pair[1], at=at + 0.5, duration=0.4, gain=gain * 0.88)
    return p

VIBES = {
    0: ("C", "up"), 2: ("G", "down"),      # a
    5: ("F", "up"), 7: ("C", "down"),      # a-prime (rotated motif: gap at 3)
    9: ("G", "down"), 11: ("G", "up"),     # b (reversed motif: gap at 0)
    12: ("C", "up"), 14: ("G", "up"),      # last-a
}
GAP_BEAT = {9: 0.0, 11: 0.0}  # the reversed motif's gap opens the bar

for bar, (chord, kind) in VIBES.items():
    answer = vibes_answer(chord, kind, at=GAP_BEAT.get(bar, 3.0))
    song.arrange(vibes, answer.humanize(timing=0.10, velocity=0.06, seed=601 + bar), bars=bar)

# --- Glockenspiel: three-note sprinkles at the phrase ends, low velocity ---
def sprinkle(p1: str, p2: str, p3: str, gain: float = 0.30) -> tono.Pattern:
    p = tono.Pattern(bars=1)
    p.note(p1, at=3.0, duration=0.25, gain=gain)
    p.note(p2, at=3.25, duration=0.25, gain=gain * 0.9)
    p.note(p3, at=3.5, duration=0.25, gain=gain * 1.08)
    return p

song.arrange(glock, sprinkle("G5", "C6", "E6"), bars=3)
song.arrange(glock, sprinkle("C6", "E6", "G6"), bars=7)
song.arrange(glock, sprinkle("B5", "D6", "G6"), bars=11)
song.arrange(glock, sprinkle("E6", "G6", "C7"), bars=15)   # crowns the C landing

# --- The melody (original): one motif, three transforms ---------------------
def phrase(bars: int, notes) -> tono.Pattern:
    p = tono.Pattern(bars=bars)
    for at, pitch, dur, gain in notes:
        p.note(pitch, at=at, duration=dur, gain=gain)
    return p

# The A motif: bar 1 bounces 8th-note skips up to the A5 color and leaves a
# one-beat gap (that's where the vibraphone answers); bar 2 peaks at C6,
# then climbs E–F–F#–G — the cheeky chromatic approach into the phrase end.
motif_a = phrase(2, [
    (0.0, "E5", 0.5, 0.80), (0.5, "G5", 0.5, 0.84), (1.0, "C5", 0.5, 0.78),
    (1.5, "E5", 0.5, 0.82), (2.0, "A5", 0.5, 0.88), (2.5, "G5", 0.5, 0.80),
    (4.0, "A5", 0.5, 0.80), (4.5, "C6", 0.5, 0.88), (5.0, "A5", 0.5, 0.82),
    (5.5, "G5", 0.5, 0.78), (6.0, "E5", 0.5, 0.74), (6.5, "F5", 0.5, 0.72),
    (7.0, "F#5", 0.5, 0.78), (7.5, "G5", 0.5, 0.88),
])

# Everything else is DERIVED from the motif — transforms as composition tools:
a_prime = motif_a.rotate(16)     # A' swaps the bars: the F-run leads over Am
b_motif = motif_a.reverse()      # B mirrors it: the chromatic climb opens up
a_last = motif_a.transpose(-7)   # A'' down a fifth — F–F#–G becomes A#–B–C,
                                 # a leading-tone landing on a clean final C

# A light, even hand on the marimba — baked in, deterministic (fixed seeds).
# gate(0.55) turns the 8th notes into the staccato pluck the brief wants.
PHRASES = [(motif_a, 0), (motif_a, 2), (a_prime, 4), (a_prime, 6),
           (b_motif, 8), (b_motif, 10), (a_last, 12), (a_last, 14)]
for i, (pat, bar) in enumerate(PHRASES):
    placed = pat.humanize(timing=0.10, velocity=0.05, seed=501 + i).gate(0.55)
    song.arrange(marimba, placed, bars=bar)

# --- Sections and the map ----------------------------------------------------
song.add_section("a", bar=0, bars=4)
song.add_section("a-prime", bar=4, bars=4)
song.add_section("b", bar=8, bars=4)
song.add_section("last-a", bar=12, bars=4)
song.add_marker("b-section", beat=32)

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
with wave.open("puzzle_menu.wav", "wb") as w:
    w.setnchannels(2)
    w.setsampwidth(2)
    w.setframerate(program.sample_rate)
    w.writeframes(pcm.tobytes())
print("wrote     puzzle_menu.wav — press play.")
