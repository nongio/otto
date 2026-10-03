"""Stage 4: a 5 s motion clip per shot via Kling 3 Pro image-to-video on fal.

Needs an approved keyframe per shot (`run.py approve keyframe s03 2`, or
--auto-approve to take the first). Cached on (model, approved keyframe,
motion prompt, duration). Native audio is off: it costs more and the edit
replaces it anyway.
"""

import hashlib

from . import common as c


def jobs(film, only=None, auto_approve=False, takes=1):
    secs = film["format"]["clip_seconds"]
    out = []
    for beat, shot in c.shots(film):
        if only and shot["id"] not in only:
            continue
        if auto_approve and not c.approved("keyframes", shot["id"]) and c.takes("keyframes", shot["id"]):
            c.approve("keyframes", shot["id"], 1)
        kf = c.approved("keyframes", shot["id"])
        kf_sha = hashlib.sha256(kf.read_bytes()).hexdigest()[:16] if kf else None
        prompt = c.motion_prompt(film, shot)
        for n in range(1, takes + 1):
            key = c.cache_key(c.VIDEO_I2V, kf_sha, prompt, secs, n)
            out.append({
                "id": shot["id"], "take": n, "keyframe": kf, "prompt": prompt, "seconds": secs,
                "path": c.BUILD / "clips" / shot["id"] / f"take{n}-{key}.mp4",
                "payload": {"prompt": prompt, "duration": str(secs), "generate_audio": False,
                            "negative_prompt": film["negative_prompt"], "cfg_scale": 0.5},
            })
    return out


def run(args, film):
    js = jobs(film, args.only, args.auto_approve, args.takes or 1)
    rate = c.PRICES[c.VIDEO_I2V]
    todo = [j for j in js if not j["path"].exists()]
    est = sum(j["seconds"] * rate for j in todo)
    c.log(f"[motion] {len(js)} clips, {len(todo)} not cached, {js[0]['seconds'] if js else 0}s each at ${rate}/s")
    for j in js:
        tag = "(cached)" if j["path"].exists() else f"${j['seconds'] * rate:.2f}"
        kf = j["keyframe"].name if j["keyframe"] else "(needs approved keyframe)"
        c.log(f"  {j['id']} take {j['take']} {tag} from {kf}")
        c.log(c.wrap(j["prompt"]))
    c.check_budget("motion", est, args.budget, args.dry_run)
    if args.dry_run or not todo:
        return
    blocked = [j["id"] for j in todo if not j["keyframe"]]
    if blocked:
        raise SystemExit(f"[motion] {blocked} need an approved keyframe (`run.py approve keyframe <shot> <take>`)")
    c.require_key("FAL_KEY", False)
    for j in todo:
        payload = dict(j["payload"], start_image_url=c.remote_or_inline(j["keyframe"]))
        res = c.fal_run(c.VIDEO_I2V, payload, poll_every=8, timeout=1800)
        c.record_spend("motion", f"{j['id']} take {j['take']}", j["seconds"] * rate)
        url = res["video"]["url"]
        j["path"].parent.mkdir(parents=True, exist_ok=True)
        c.download(url, j["path"])
        c.write_meta(j["path"], {"remote_url": url, "prompt": j["prompt"], "keyframe": str(j["keyframe"])})
        c.log(f"  {j['id']} -> {j['path'].relative_to(c.ROOT)}")
