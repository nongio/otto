"""Stage 6: contact sheets and the measured rules from the article.

Contact sheets: 8 frames per shot sampled from the final file, and 4 fps of
every motion clip. Checks: length, median shot, gaps, ending, speech rate,
voice over music, integrated loudness, caption length, script lint.
Writes build/qa/report.json and exits non-zero on a failed check.
"""

import json
import re
import statistics
import subprocess

from . import common as c
from . import edit as e

BANNED = ["macos", "mac os", "windows 1", "microsoft", "apple", "gnome", "kde", "plasma", "ubuntu", "chromeos"]


def lint(film):
    """Script checks that need no generated asset."""
    problems = []
    for beat in film["beats"]:
        for cap in beat.get("captions", []):
            n = len(cap.split())
            if not 2 <= n <= 4:
                problems.append(f"{beat['id']}: caption '{cap}' has {n} words (want 2-4)")
        for shot in beat["shots"]:
            if not shot["refs"] and "no living thing" not in shot["prompt"].lower():
                problems.append(f"{shot['id']}: empty scene must say 'no living thing'")
            for r in shot["refs"]:
                if r not in film["characters"]:
                    problems.append(f"{shot['id']}: unknown character ref '{r}'")
            try:
                c.expand(shot["prompt"] + shot["motion"], film)
            except KeyError as err:
                problems.append(f"{shot['id']}: {err}")
    text = " ".join(b["vo"] + " " + " ".join(b.get("captions", [])) for b in film["beats"]).lower()
    for w in BANNED:
        if re.search(r"\b" + re.escape(w) + r"\b", text):
            problems.append(f"script mentions '{w}': no comparisons with other desktops")
    dashes = text.count("—")
    if dashes > 2:
        problems.append(f"{dashes} em dashes in narration/captions; keep it to a couple")
    return problems


def silences(path, start, end, min_len=0.75, noise="-40dB"):
    err = subprocess.run(["ffmpeg", "-hide_banner", "-nostats", "-i", str(path), "-af",
                          f"silencedetect=noise={noise}:d={min_len}", "-f", "null", "-"],
                         capture_output=True, text=True).stderr
    starts = [float(x) for x in re.findall(r"silence_start: (-?[\d.]+)", err)]
    ends = [float(x) for x in re.findall(r"silence_end: ([\d.]+)", err)]
    ends += [end] * (len(starts) - len(ends))
    return [(max(s, start), min(t, end)) for s, t in zip(starts, ends) if min(t, end) - max(s, start) >= min_len]


def contact_sheets(film, tl):
    qa = c.stage_dir("qa")
    rows = []
    for shot in tl["shots"]:
        row = qa / f"final_{shot['id']}.png"
        d = max(shot["dur"], 0.01)
        c.run(["ffmpeg", "-y", "-v", "error", "-ss", f"{shot['start']:.3f}", "-t", f"{d:.3f}", "-i", str(e.OUT),
               "-vf", f"fps=8/{d:.3f},scale=216:384,tile=8x1:padding=4:color=0x1a1410", "-frames:v", "1", str(row)])
        rows.append(row)
    sheet = qa / "contact_final.png"
    if rows:
        ins = sum((["-i", str(r)] for r in rows), [])
        c.run(["ffmpeg", "-y", "-v", "error", *ins, "-filter_complex",
               f"{''.join(f'[{i}:v]' for i in range(len(rows)))}vstack=inputs={len(rows)}", str(sheet)])
    clip_sheets = []
    for shot in tl["shots"]:
        kind, src = e.shot_source(film, shot["id"])
        if kind != "clip":
            continue
        out = qa / f"clip_{shot['id']}.png"
        c.run(["ffmpeg", "-y", "-v", "error", "-i", str(src), "-vf",
               "fps=4,scale=216:384,tile=10x2:padding=4:color=0x1a1410", "-frames:v", "1", str(out)])
        clip_sheets.append(out)
    return sheet, clip_sheets


def run(args, film):
    tl_path = c.BUILD / "edit" / "timeline.json"
    if not e.OUT.exists() or not tl_path.exists():
        raise SystemExit("[qa] no edit yet; run `run.py edit` first")
    tl = json.loads(tl_path.read_text())
    checks = []

    def check(name, ok, value, rule):
        checks.append({"check": name, "ok": bool(ok), "value": value, "rule": rule})

    total = c.ffprobe_duration(e.OUT)
    lo, hi = film["format"]["target_seconds"]
    check("length", lo <= total <= hi, f"{total:.2f}s", f"{lo}-{hi}s")

    durs = [s["dur"] for s in tl["shots"]]
    med = statistics.median(durs)
    check("median shot", 2.0 <= med <= 2.9, f"{med:.2f}s", "about 2.6s (2.0-2.9)")

    last = tl["beats"][-1]
    check("ending", last["end"] - last["start"] <= 6.0, f"{last['end'] - last['start']:.2f}s", "<= 6s")

    words = sum(b["words"] for b in tl["beats"])
    if tl["has_voice"]:
        speech = sum(b["speech_end"] - b["start"] for b in tl["beats"])
        check("speech rate", words / speech >= 3.0, f"{words / speech:.2f} w/s", ">= 3 words/s")
        slow = [b["id"] for b in tl["beats"] if b["words"] / (b["speech_end"] - b["start"]) < 2.7]
        check("no slow beats", not slow, slow or "none", "every beat >= 2.7 w/s")
        voice = c.BUILD / "edit" / "voice_stem.wav"
        gaps = silences(voice, 0.0, total)
        check("gaps", not gaps, [f"{a:.2f}-{b:.2f}" for a, b in gaps] or "none", "no silence > 0.75s")
        tail = total - last["speech_end"]
        check("tail after last word", tail <= 0.75, f"{tail:.2f}s", "<= 0.75s")
        v = c.loudness(voice)
        m = c.loudness(c.BUILD / "edit" / "music_stem.wav", 0, last["speech_end"])
        check("voice over music", 13 <= v - m <= 26, f"{v - m:.1f} dB", "13-26 dB")
    else:
        cap_gaps = []
        prev = 0.0
        for cap in tl["captions"]:
            if cap["start"] - prev > 0.75:
                cap_gaps.append(f"{prev:.2f}-{cap['start']:.2f}")
            prev = cap["end"]
        check("caption gaps (no voice)", not cap_gaps, cap_gaps or "none", "no caption gap > 0.75s")

    lufs = c.loudness(e.OUT)
    check("loudness", -15 <= lufs <= -13, f"{lufs:.1f} LUFS", "-15 to -13 LUFS")

    problems = lint(film)
    check("script lint", not problems, problems or "clean", "captions 2-4 words, empty scenes, no comparisons")

    placeholders = [s["id"] for s in tl["shots"] if s.get("source") != "clip"]
    check("all shots are motion clips", not placeholders, placeholders or "all clips", "every shot generated")

    sheet, clip_sheets = contact_sheets(film, tl)
    manual = [
        "Opening shot is a concrete physical action, not a title or logo (b01/s01)",
        "Second line introduces something new (b02: Otto)",
        "Moss looks the same in every shot (contact_final.png)",
        "Only features that exist are shown: genie minimise, Exposé, workspace swipe, tiling, island music",
    ]
    report = {"file": str(e.OUT), "checks": checks, "manual": manual,
              "contact_final": str(sheet), "contact_clips": [str(p) for p in clip_sheets]}
    (c.BUILD / "qa" / "report.json").write_text(json.dumps(report, indent=2))

    c.log(f"[qa] {e.OUT.relative_to(c.ROOT)}")
    for ch in checks:
        c.log(f"  {'PASS' if ch['ok'] else 'FAIL'}  {ch['check']:<26} {ch['value']}  ({ch['rule']})")
    c.log("  human checks:")
    for m in manual:
        c.log(f"    [ ] {m}")
    c.log(f"  contact sheets: {sheet.relative_to(c.ROOT)}"
          + (f" + {len(clip_sheets)} clip sheets" if clip_sheets else ""))
    failed = [ch["check"] for ch in checks if not ch["ok"]]
    if failed and not args.allow_fail:
        raise SystemExit(f"[qa] failed: {failed}")
