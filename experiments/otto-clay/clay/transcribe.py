"""Stage 2b: transcribe each narration clip with Whisper and diff it against the script.

Uses a local Whisper (faster-whisper or openai-whisper) when one is
installed, otherwise fal-ai/whisper (paid, a cent or so per beat).
Writes build/voice/transcript.json and fails if any beat drifts.
"""

import difflib
import json
import re

from . import common as c

MAX_WER = 0.10


def words(text):
    text = text.lower().replace("-", " ")
    return re.findall(r"[a-z0-9']+", text)


def wer(ref, hyp):
    r, h = words(ref), words(hyp)
    sm = difflib.SequenceMatcher(a=r, b=h, autojunk=False)
    errors = sum(max(i2 - i1, j2 - j1) for op, i1, i2, j1, j2 in sm.get_opcodes() if op != "equal")
    return errors / max(1, len(r))


def local_backend():
    try:
        from faster_whisper import WhisperModel

        model = WhisperModel("small.en")
        return lambda p: " ".join(s.text for s in model.transcribe(str(p), language="en")[0])
    except ImportError:
        pass
    try:
        import whisper

        model = whisper.load_model("small.en")
        return lambda p: model.transcribe(str(p), language="en")["text"]
    except ImportError:
        return None


def run(args, film):
    if args.no_voice:
        c.log("[transcribe] --no-voice: nothing to transcribe")
        return
    clips = [(b, c.BUILD / "voice" / f"{b['id']}.mp3") for b in film["beats"]]
    missing = [b["id"] for b, p in clips if not p.exists()]
    out_path = c.BUILD / "voice" / "transcript.json"
    cache = json.loads(out_path.read_text()) if out_path.exists() else {}
    todo = [(b, p) for b, p in clips if p.exists() and cache.get(b["id"], {}).get("key") != (c.read_meta(p) or {}).get("key")]

    import importlib.util

    has_local = any(importlib.util.find_spec(m) for m in ("faster_whisper", "whisper"))
    backend = local_backend() if has_local and not args.dry_run else None
    paid = not has_local
    est = len(todo) * c.PRICES[c.WHISPER] if paid else 0.0
    c.log(f"[transcribe] {len(todo)} clips to check via {'fal-ai/whisper' if paid else 'local Whisper'}"
          + (f"; {len(missing)} beats have no voice yet: {missing}" if missing else ""))
    if paid:
        c.check_budget("transcribe", est, args.budget, args.dry_run)
    if args.dry_run:
        return
    if paid and todo:
        c.require_key("FAL_KEY", False)
    for beat, mp3 in todo:
        if paid:
            res = c.fal_run(c.WHISPER, {"audio_url": c.data_uri(mp3), "task": "transcribe", "language": "en"})
            c.record_spend("transcribe", beat["id"], c.PRICES[c.WHISPER])
            text = res["text"]
        else:
            text = backend(mp3)
        cache[beat["id"]] = {"key": (c.read_meta(mp3) or {}).get("key"), "script": beat["vo"], "heard": text.strip(),
                             "wer": round(wer(beat["vo"], text), 3)}
    out_path.parent.mkdir(parents=True, exist_ok=True)
    out_path.write_text(json.dumps(cache, indent=2))
    bad = []
    for beat, _ in clips:
        r = cache.get(beat["id"])
        if not r:
            continue
        flag = "OK " if r["wer"] <= MAX_WER else "BAD"
        c.log(f"  {flag} {beat['id']} wer={r['wer']:.2f}  heard: {r['heard']}")
        if r["wer"] > MAX_WER:
            bad.append(beat["id"])
    if bad:
        raise SystemExit(f"[transcribe] narration drifted from the script in {bad}; delete those mp3s and rerun voice")
