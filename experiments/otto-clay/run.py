#!/usr/bin/env python3
"""otto-clay: one command per stage of the clay-film pipeline.

    ./run.py plan                      every prompt + estimated spend, calls nothing
    ./run.py voice     [--dry-run]     ElevenLabs narration per beat
    ./run.py transcribe                Whisper the narration back, diff against the script
    ./run.py sheet     [--takes 3]     character reference sheet candidates
    ./run.py approve sheet moss 2      pick a take (also: approve keyframe s03 1, approve clip s03 1)
    ./run.py keyframes [--takes 2]     keyframes, sheet attached to every shot
    ./run.py motion                    5 s Kling clips from the approved keyframes
    ./run.py edit      [--no-voice]    cut to the narration, captions, music, loudness
    ./run.py qa                        contact sheets + measured checks
    ./run.py all       [--auto-approve]  everything in order
    ./run.py status                    what exists, what is approved, what was spent

Paid stages print their estimate first, refuse to exceed --budget (default
$15 for the whole film, counted against build/ledger.json), and never run
without FAL_KEY / ELEVENLABS_API_KEY.
"""

import argparse
import sys

from clay import common as c
from clay import edit, images, motion, music, qa, transcribe, voice


def estimate_all(film, args):
    """Cost of a whole first pass, ignoring the cache: (rows, total)."""
    sheet_takes = args.takes or 3
    kf_takes = args.takes or 2
    n_chars = len(film["characters"])
    n_shots = sum(1 for _ in c.shots(film))
    chars = sum(len(b["vo"]) for b in film["beats"])
    secs = film["format"]["clip_seconds"]
    rows = [
        ("voice (ElevenLabs)", 0.0 if args.no_voice else chars / 1000 * c.PRICES["elevenlabs_per_1k_chars"],
         "skipped (--no-voice)" if args.no_voice else f"{chars} chars x ${c.PRICES['elevenlabs_per_1k_chars']}/1k"),
        ("transcribe (Whisper)", 0.0, "local Whisper free; fal-ai/whisper fallback about $0.01/beat"),
        ("character sheet", n_chars * sheet_takes * c.PRICES[c.IMAGE_T2I],
         f"{n_chars} x {sheet_takes} takes x ${c.PRICES[c.IMAGE_T2I]}"),
        ("keyframes (Nano Banana)", n_shots * kf_takes * c.PRICES[c.IMAGE_EDIT],
         f"{n_shots} shots x {kf_takes} takes x ${c.PRICES[c.IMAGE_EDIT]}"),
        ("motion (Kling 3 Pro)", n_shots * secs * c.PRICES[c.VIDEO_I2V],
         f"{n_shots} clips x {secs}s x ${c.PRICES[c.VIDEO_I2V]}/s"),
        ("music, edit, QA", 0.0, "local (synth + ffmpeg)"),
    ]
    return rows, sum(r[1] for r in rows)


def cmd_plan(args, film):
    args.dry_run = True
    c.log(f"# {film['title']}\n{film['logline']}\n")
    problems = qa.lint(film)
    c.log("[lint] " + ("clean" if not problems else "\n  ".join(["problems:"] + problems)))
    tl = edit.build_timeline(film, no_voice=True)
    c.log(f"[timing] {len(tl['shots'])} shots, about {tl['total']:.1f}s at "
          f"{film['edit']['no_voice_words_per_second']} words/s (real timing comes from the narration)\n")
    c.log("[style prefix]\n" + c.wrap(film["style_prefix"]) + "\n")
    voice.run(args, film)
    c.log("")
    images.run_sheets(args, film)
    c.log("")
    images.run_keyframes(args, film)
    c.log("")
    motion.run(args, film)
    c.log("")
    rows, total = estimate_all(film, args)
    c.log("[estimate] first pass, nothing cached:")
    for name, usd, how in rows:
        c.log(f"  {name:<26} ${usd:6.2f}   {how}")
    c.log(f"  {'TOTAL':<26} ${total:6.2f}   budget ${args.budget:.2f}, "
          f"leaves ${args.budget - total:.2f} for retakes (the article lost 20% to rejected takes)")
    if total > args.budget:
        c.log("  over budget: trim takes or shots before generating")


def cmd_approve(args, film):
    kind = {"sheet": "sheets", "keyframe": "keyframes", "clip": "clips"}[args.kind]
    c.approve(kind, args.ident, args.take)


def cmd_status(args, film):
    c.log(f"spent so far: ${c.spent():.2f} of ${args.budget:.2f}")
    for cid in film["characters"]:
        c.log(f"sheet {cid}: {len(c.takes('sheets', cid))} takes, approved={bool(c.approved('sheets', cid))}")
    for beat, shot in c.shots(film):
        v = (c.BUILD / "voice" / f"{beat['id']}.mp3").exists()
        kind, _ = edit.shot_source(film, shot["id"])
        c.log(f"{beat['id']} {shot['id']}: voice={'yes' if v else 'no '} "
              f"keyframes={len(c.takes('keyframes', shot['id']))} "
              f"approved={'yes' if c.approved('keyframes', shot['id']) else 'no '} edit uses {kind}")


def cmd_all(args, film):
    rows, total = estimate_all(film, args)
    remaining = args.budget - c.spent()
    c.log(f"[all] a full first pass costs about ${total:.2f} uncached; ${remaining:.2f} of budget left")
    stages = [voice.run, transcribe.run, images.run_sheets, images.run_keyframes, motion.run, edit.run, qa.run]
    for stage in stages:
        if args.dry_run and stage in (edit.run, qa.run):
            continue
        stage(args, film)


def main(argv):
    p = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = p.add_subparsers(dest="cmd", required=True)

    def add(name, fn, **kw):
        sp = sub.add_parser(name, **kw)
        c.add_common_args(sp)
        sp.add_argument("--no-voice", action="store_true", help="captions only, no narration")
        sp.add_argument("--takes", type=int, help="candidates per sheet/keyframe/clip")
        sp.add_argument("--only", type=lambda s: s.split(","), help="comma-separated shot or character ids")
        sp.add_argument("--auto-approve", action="store_true", help="treat take 1 as approved")
        sp.add_argument("--scratch", action="store_true",
                        help="voice/edit: free espeak-ng narration to check timing before paying")
        sp.add_argument("--allow-fail", action="store_true", help="qa: report failures without exiting non-zero")
        sp.set_defaults(fn=fn)
        return sp

    add("plan", cmd_plan)
    add("voice", voice.run)
    add("transcribe", transcribe.run)
    add("sheet", images.run_sheets)
    add("keyframes", images.run_keyframes)
    add("motion", motion.run)
    add("music", lambda a, f: music.run(a, f))
    add("edit", edit.run)
    add("qa", qa.run)
    add("all", cmd_all)
    add("status", cmd_status)
    ap = add("approve", cmd_approve)
    ap.add_argument("kind", choices=["sheet", "keyframe", "clip"])
    ap.add_argument("ident")
    ap.add_argument("take", type=int)

    args = p.parse_args(argv)
    film = c.load_film(args.film)
    try:
        args.fn(args, film)
    except c.BudgetExceeded as err:
        c.die(str(err))


if __name__ == "__main__":
    main(sys.argv[1:])
