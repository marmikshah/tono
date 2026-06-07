#!/usr/bin/env python3
"""Parse an SMF MIDI file and emit Sonarium seq notes (JSON)."""
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
    raw = open(path, "rb").read()
    assert raw[:4] == b"MThd"
    _, fmt, ntrk, division = struct.unpack(">IHHH", raw[4:14])
    i = 14
    tracks = []
    tempo = 500000  # default us/quarter
    for _ in range(ntrk):
        assert raw[i : i + 4] == b"MTrk"
        length = struct.unpack(">I", raw[i + 4 : i + 8])[0]
        data = raw[i + 8 : i + 8 + length]
        i += 8 + length
        j, t, status = 0, 0, 0
        notes, active = [], {}
        while j < len(data):
            delta, j = read_varlen(data, j)
            t += delta
            b = data[j]
            if b & 0x80:
                status = b
                j += 1
            ev = status & 0xF0
            if ev == 0x90 and data[j + 1] > 0:  # note on
                active.setdefault(data[j], []).append((t, data[j + 1]))
                j += 2
            elif ev == 0x80 or (ev == 0x90 and data[j + 1] == 0):  # note off
                if active.get(data[j]):
                    start, vel = active[data[j]].pop(0)
                    notes.append((start, data[j], vel, t - start))
                j += 2
            elif ev in (0xA0, 0xB0, 0xE0):
                j += 2
            elif ev in (0xC0, 0xD0):
                j += 1
            elif status == 0xFF:  # meta
                mtype = data[j]
                mlen, j2 = read_varlen(data, j + 1)
                if mtype == 0x51:
                    tempo = int.from_bytes(data[j2 : j2 + 3], "big")
                j = j2 + mlen
            elif status in (0xF0, 0xF7):  # sysex
                mlen, j2 = read_varlen(data, j)
                j = j2 + mlen
        tracks.append(notes)
    return division, tempo, tracks


def main():
    path, max_secs = sys.argv[1], float(sys.argv[2])
    division, tempo, tracks = parse(path)
    bpm = 60_000_000 / tempo
    ticks_per_step = division / 4  # 16th-note grid
    secs_per_tick = tempo / 1_000_000 / division

    out_tracks = []
    for notes in tracks:
        seq = []
        for start, pitch, vel, dur in sorted(notes):
            if start * secs_per_tick > max_secs:
                continue
            step = round(start / ticks_per_step)
            length = max(1, round(dur / ticks_per_step))
            seq.append(
                {
                    "step": step,
                    "len": length,
                    "pitch": f"midi:{pitch}",
                    "gain": round(min(1.0, vel / 110), 2),
                }
            )
        if seq:
            out_tracks.append(seq)
    print(
        json.dumps(
            {
                "bpm": round(bpm, 2),
                "division": division,
                "n_tracks": len(out_tracks),
                "counts": [len(t) for t in out_tracks],
                "tracks": out_tracks,
            }
        )
    )


if __name__ == "__main__":
    main()
