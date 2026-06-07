#!/usr/bin/env python3
"""Convert a Standard MIDI File into Sonarium `seq` notes.

Faithful conversion for replicas:
- applies the full tempo map (rubato survives — note times are computed in
  real seconds, not one global bpm),
- honours the sustain pedal (CC64): while the pedal is down, releases are
  deferred to the pedal-up, like a real piano,
- quantizes onto a fine fixed grid (default 20 ms) so the seq timing matches
  the performance to within one grid step.

Usage: midi_to_seq.py FILE.mid MAX_SECS [SPLIT_MIDI_PITCH]
Emits JSON: { bpm, steps_per_beat, counts, tracks: { low, high } } where
notes split at SPLIT_MIDI_PITCH (default 60) into the two hands.
"""
import json
import struct
import sys


def read_varlen(data, i):
    val = 0
    while True:
        b = data[i]
        i += 1
        val = (val << 7) | (b & 0x7F)
        if not b & 0x80:
            return val, i


def parse(path):
    """Return (division, [(abs_tick, kind, a, b)]) merged across tracks.

    Event kinds: ("tempo", us_per_quarter), ("on", pitch, vel),
    ("off", pitch), ("pedal", down).
    """
    raw = open(path, "rb").read()
    assert raw[:4] == b"MThd", "not a MIDI file"
    _, _fmt, ntrk, division = struct.unpack(">IHHH", raw[4:14])
    i = 14
    events = []
    for _ in range(ntrk):
        assert raw[i : i + 4] == b"MTrk"
        length = struct.unpack(">I", raw[i + 4 : i + 8])[0]
        data = raw[i + 8 : i + 8 + length]
        i += 8 + length
        j, t, status = 0, 0, 0
        while j < len(data):
            delta, j = read_varlen(data, j)
            t += delta
            b = data[j]
            if b & 0x80:
                status = b
                j += 1
            ev = status & 0xF0
            if ev == 0x90 and data[j + 1] > 0:
                events.append((t, "on", data[j], data[j + 1]))
                j += 2
            elif ev == 0x80 or (ev == 0x90 and data[j + 1] == 0):
                events.append((t, "off", data[j], 0))
                j += 2
            elif ev == 0xB0:
                if data[j] == 64:  # sustain pedal
                    events.append((t, "pedal", int(data[j + 1] >= 64), 0))
                j += 2
            elif ev in (0xA0, 0xE0):
                j += 2
            elif ev in (0xC0, 0xD0):
                j += 1
            elif status == 0xFF:
                mtype = data[j]
                mlen, j2 = read_varlen(data, j + 1)
                if mtype == 0x51:
                    tempo = int.from_bytes(data[j2 : j2 + 3], "big")
                    events.append((t, "tempo", tempo, 0))
                j = j2 + mlen
            elif status in (0xF0, 0xF7):
                mlen, j2 = read_varlen(data, j)
                j = j2 + mlen
    events.sort(key=lambda e: e[0])
    return division, events


def to_seconds(division, events):
    """Walk the tempo map: attach a real-time second to every event."""
    out, tempo, last_tick, last_sec = [], 500000, 0, 0.0
    for tick, kind, a, b in events:
        last_sec += (tick - last_tick) * tempo / 1_000_000 / division
        last_tick = tick
        if kind == "tempo":
            tempo = a
        else:
            out.append((last_sec, kind, a, b))
    return out


def notes_with_pedal(timed):
    """Note (start_sec, pitch, vel, end_sec) list, releases held by pedal."""
    notes, active, pedal, deferred = [], {}, False, []
    for sec, kind, a, b in timed:
        if kind == "on":
            active.setdefault(a, []).append((sec, b))
        elif kind == "off":
            if active.get(a):
                start, vel = active[a].pop(0)
                if pedal:
                    deferred.append((start, a, vel))
                else:
                    notes.append((start, a, vel, sec))
        elif kind == "pedal":
            pedal = bool(a)
            if not pedal:
                for start, pitch, vel in deferred:
                    notes.append((start, pitch, vel, sec))
                deferred = []
    # Anything still sounding ends at the last event.
    last = timed[-1][0] if timed else 0.0
    for start, pitch, vel in deferred:
        notes.append((start, pitch, vel, last))
    for pitch, stack in active.items():
        for start, vel in stack:
            notes.append((start, pitch, vel, last))
    notes.sort()
    return notes


def main():
    path = sys.argv[1]
    max_secs = float(sys.argv[2])
    split = int(sys.argv[3]) if len(sys.argv) > 3 else 60

    division, events = parse(path)
    notes = notes_with_pedal(to_seconds(division, events))

    # Fixed fine grid: 20 ms steps (bpm 60 × 50 steps/beat) keeps the tempo
    # map's rubato to within one step.
    bpm, spb = 60.0, 50
    step_secs = 60.0 / bpm / spb

    tracks = {"low": [], "high": []}
    for start, pitch, vel, end in notes:
        if start > max_secs:
            continue
        hand = "high" if pitch >= split else "low"
        tracks[hand].append(
            {
                "step": round(start / step_secs),
                "len": max(1, round((end - start) / step_secs)),
                "pitch": f"midi:{pitch}",
                "gain": round(min(1.0, vel / 105), 2),
            }
        )
    print(
        json.dumps(
            {
                "bpm": bpm,
                "steps_per_beat": spb,
                "counts": {k: len(v) for k, v in tracks.items()},
                "tracks": tracks,
            }
        )
    )


if __name__ == "__main__":
    main()
