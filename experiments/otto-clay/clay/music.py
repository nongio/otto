"""Stage 5a: a royalty-free music bed, synthesised here so there is nothing to license.

Plucked strings (Karplus-Strong) arpeggiating I-V-vi-IV, a round sine bass,
a toy glockenspiel hook and a soft shaker. Pure Python, deterministic.
Writes build/music/bed.wav.
"""

import array
import math
import random
import wave

from . import common as c

SR = 44100
NOTES = {"C": 0, "D": 2, "E": 4, "F": 5, "G": 7, "A": 9, "B": 11}


def midi(name, octave):
    return 12 * (octave + 1) + NOTES[name[0]] + (1 if name.endswith("#") else 0)


def hz(m):
    return 440.0 * 2 ** ((m - 69) / 12)


# I-V-vi-IV in C, one chord per bar, as (root, triad) MIDI numbers.
CHORDS = [
    (midi("C", 2), [midi("C", 4), midi("E", 4), midi("G", 4), midi("C", 5)]),
    (midi("G", 2), [midi("B", 3), midi("D", 4), midi("G", 4), midi("B", 4)]),
    (midi("A", 2), [midi("C", 4), midi("E", 4), midi("A", 4), midi("C", 5)]),
    (midi("F", 2), [midi("C", 4), midi("F", 4), midi("A", 4), midi("C", 5)]),
]
# Glockenspiel hook over two bars (eighth-note steps; None is a rest).
HOOK = [midi("E", 5), None, midi("G", 5), None, midi("A", 5), midi("G", 5), None, None,
        midi("E", 5), None, midi("D", 5), None, midi("C", 5), None, None, None]
ARP = [0, 2, 1, 3, 2, 1, 3, 2]


def pluck(freq, secs, gain, rng):
    n = int(SR / freq)
    buf = [rng.uniform(-1, 1) for _ in range(n)]
    out = []
    i = 0
    for _ in range(int(secs * SR)):
        a = buf[i % n]
        b = buf[(i + 1) % n]
        v = 0.497 * (a + b)
        buf[i % n] = v
        out.append(a * gain)
        i += 1
    return out


def sine(freq, secs, gain, decay):
    w = 2 * math.pi * freq / SR
    return [gain * math.sin(w * t) * math.exp(-decay * t / SR) * min(1.0, t / 200) for t in range(int(secs * SR))]


def glock(freq, secs, gain):
    a = sine(freq, secs, gain, 6.0)
    b = sine(freq * 2.76, secs, gain * 0.25, 14.0)
    return [x + y for x, y in zip(a, b)]


def shaker(secs, gain, rng):
    out, prev = [], 0.0
    for t in range(int(secs * SR)):
        x = rng.uniform(-1, 1)
        hp = x - prev
        prev = x
        out.append(hp * gain * math.exp(-40 * t / SR))
    return out


def mix_into(track, sig, at, pan):
    start = int(at * SR)
    lg, rg = math.cos(pan * math.pi / 2), math.sin(pan * math.pi / 2)
    for k, v in enumerate(sig):
        j = start + k
        if j >= len(track) // 2:
            break
        track[2 * j] += v * lg
        track[2 * j + 1] += v * rg


def synth(seconds, bpm, seed=7):
    rng = random.Random(seed)
    beat = 60.0 / bpm
    eighth = beat / 2
    total = int(seconds * SR)
    track = [0.0] * (2 * total)
    bar = 0
    t = 0.0
    while t < seconds:
        root, triad = CHORDS[bar % 4]
        for step in range(8):
            at = t + step * eighth
            if at >= seconds:
                break
            mix_into(track, pluck(hz(triad[ARP[step]]), 0.9, 0.22, rng), at, 0.35)
            if step in (0, 4):
                mix_into(track, sine(hz(root), beat * 1.8, 0.34, 3.0), at, 0.5)
            if step % 2 == 1:
                mix_into(track, shaker(0.08, 0.05, rng), at, 0.7)
            # The hook comes in from bar 3 so the opening stays sparse.
            h = HOOK[(bar % 2) * 8 + step]
            if bar >= 2 and h:
                mix_into(track, glock(hz(h), 0.8, 0.12), at, 0.6)
        bar += 1
        t += 4 * beat
    # Fade out the last 1.5 s, soft-clip, write.
    fade = int(1.5 * SR)
    for j in range(max(0, total - fade), total):
        g = (total - j) / fade
        track[2 * j] *= g
        track[2 * j + 1] *= g
    return array.array("h", (int(32767 * math.tanh(v)) for v in track))


def run(args, film, seconds=None):
    seconds = seconds or film["format"]["target_seconds"][1] + 2
    bpm = film["music"]["bpm"]
    out = c.stage_dir("music") / "bed.wav"
    key = c.cache_key("music-v1", seconds, bpm)
    if out.exists() and (c.read_meta(out) or {}).get("key") == key:
        c.log(f"[music] cached {out.relative_to(c.ROOT)}")
        return out
    c.log(f"[music] synthesising {seconds:.1f}s at {bpm} bpm (free, local)")
    pcm = synth(seconds, bpm)
    with wave.open(str(out), "wb") as w:
        w.setnchannels(2)
        w.setsampwidth(2)
        w.setframerate(SR)
        w.writeframes(pcm.tobytes())
    c.write_meta(out, {"key": key})
    return out
