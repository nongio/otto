"""Stage 3: character reference sheets, then keyframes with the sheet attached.

Sheets: fal-ai/nano-banana (text to image), --takes candidates per character,
a human approves one (`run.py approve sheet moss 2`).
Keyframes: fal-ai/nano-banana/edit with every referenced character's approved
sheet attached; shots with no characters use plain text to image.
Each take is cached on (model, prompt, attached sheets, take number).
"""

import hashlib

from . import common as c

SHEET_NOTE = (
    "Match the attached character reference sheet exactly: same face, colours, glasses, cardigan, "
    "buttons and proportions. Show each character only once in the scene. Do not copy the sheet's "
    "layout, background or multiple poses."
)


def file_sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()[:16]


def seed_for(key):
    return int(key[:8], 16) % (2**31)


def _take_path(kind, ident, n, key):
    return c.BUILD / kind / ident / f"take{n}-{key}.png"


def sheet_jobs(film, takes, only=None):
    jobs = []
    for cid in film["characters"]:
        if only and cid not in only:
            continue
        prompt = c.sheet_prompt(film, cid)
        for n in range(1, takes + 1):
            key = c.cache_key(c.IMAGE_T2I, prompt, "16:9", n)
            payload = {"prompt": prompt, "aspect_ratio": "16:9", "num_images": 1, "output_format": "png",
                       "seed": seed_for(key)}
            jobs.append({"kind": "sheets", "id": cid, "take": n, "model": c.IMAGE_T2I, "payload": payload,
                         "path": _take_path("sheets", cid, n, key), "prompt": prompt})
    return jobs


def keyframe_jobs(film, takes, only=None, auto_approve=False):
    jobs = []
    for beat, shot in c.shots(film):
        if only and shot["id"] not in only:
            continue
        prompt = c.shot_prompt(film, shot)
        sheets, missing = [], []
        for cid in shot["refs"]:
            if auto_approve and not c.approved("sheets", cid) and c.takes("sheets", cid):
                c.approve("sheets", cid, 1)
            p = c.approved("sheets", cid)
            (sheets if p else missing).append(p or cid)
        if shot["refs"]:
            prompt = f"{prompt}\n\n{SHEET_NOTE}"
            model = c.IMAGE_EDIT
        else:
            model = c.IMAGE_T2I
        for n in range(1, takes + 1):
            key = c.cache_key(model, prompt, "9:16", [file_sha(p) for p in sheets], missing, n)
            payload = {"prompt": prompt, "aspect_ratio": "9:16", "num_images": 1, "output_format": "png",
                       "seed": seed_for(key)}
            jobs.append({"kind": "keyframes", "id": shot["id"], "take": n, "model": model, "payload": payload,
                         "sheets": sheets, "missing": missing, "path": _take_path("keyframes", shot["id"], n, key),
                         "prompt": prompt})
    return jobs


def _execute(jobs, stage, args, film):
    todo = [j for j in jobs if not j["path"].exists()]
    est = sum(c.PRICES[j["model"]] for j in todo)
    c.log(f"[{stage}] {len(jobs)} takes, {len(todo)} not cached, ${c.PRICES[c.IMAGE_T2I]}/image")
    shown = set()
    for j in jobs:
        tag = "(cached)" if j["path"].exists() else f"${c.PRICES[j['model']]:.3f}"
        extra = ""
        if j.get("sheets") or j.get("missing"):
            names = [p.parent.name for p in j.get("sheets", [])] + [f"{m} (needs approved sheet)" for m in j.get("missing", [])]
            extra = f" + sheets: {', '.join(names)}"
        c.log(f"  {j['id']} take {j['take']} {tag} via {j['model']}{extra}")
        if j["id"] not in shown:
            shown.add(j["id"])
            c.log(c.wrap(j["prompt"].replace(film["style_prefix"], "[STYLE PREFIX]")))
    c.check_budget(stage, est, args.budget, args.dry_run)
    if args.dry_run or not todo:
        return
    blocked = [j["id"] for j in todo if j.get("missing")]
    if blocked:
        raise SystemExit(f"[{stage}] shots {sorted(set(blocked))} need an approved character sheet first "
                         f"(`run.py approve sheet <char> <take>`, or --auto-approve)")
    c.require_key("FAL_KEY", False)
    for j in todo:
        payload = dict(j["payload"])
        if j.get("sheets"):
            payload["image_urls"] = [c.remote_or_inline(p) for p in j["sheets"]]
        res = c.fal_run(j["model"], payload)
        c.record_spend(stage, f"{j['id']} take {j['take']}", c.PRICES[j["model"]])
        url = res["images"][0]["url"]
        j["path"].parent.mkdir(parents=True, exist_ok=True)
        c.download(url, j["path"])
        c.write_meta(j["path"], {"remote_url": url, "model": j["model"], "prompt": j["prompt"],
                                 "description": res.get("description")})
        c.log(f"  {j['id']} take {j['take']} -> {j['path'].relative_to(c.ROOT)}")


def run_sheets(args, film):
    _execute(sheet_jobs(film, args.takes or 3, args.only), "sheet", args, film)
    for cid in film["characters"]:
        if not c.approved("sheets", cid) and c.takes("sheets", cid):
            c.log(f"  next: look at build/sheets/{cid}/ and run `run.py approve sheet {cid} <take>`")


def run_keyframes(args, film):
    _execute(keyframe_jobs(film, args.takes or 2, args.only, args.auto_approve), "keyframes", args, film)
