"""Shared plumbing: film loading, prompt expansion, caching, budget, HTTP.

Standard library only. Every generated asset lives under build/ keyed by a
hash of what produced it, so a rerun with the same inputs never pays twice.
"""

import base64
import hashlib
import json
import os
import re
import subprocess
import sys
import time
import urllib.error
import urllib.request
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
BUILD = ROOT / "build"
FILM_PATH = ROOT / "film.json"

# Prices in USD, checked on fal.ai / elevenlabs.io in October 2026.
PRICES = {
    "fal-ai/nano-banana": 0.039,  # per image
    "fal-ai/nano-banana/edit": 0.039,  # per image
    "fal-ai/kling-video/v3/pro/image-to-video": 0.112,  # per second, audio off
    "fal-ai/whisper": 0.01,  # per beat, rough upper bound (billed by compute time)
    # Credits cost depends on the plan; this is the overage rate on Creator.
    "elevenlabs_per_1k_chars": 0.30,
}

IMAGE_T2I = "fal-ai/nano-banana"
IMAGE_EDIT = "fal-ai/nano-banana/edit"
VIDEO_I2V = "fal-ai/kling-video/v3/pro/image-to-video"
WHISPER = "fal-ai/whisper"

DEFAULT_BUDGET = 15.0


class BudgetExceeded(SystemExit):
    pass


def log(msg=""):
    print(msg, flush=True)


# ---------------------------------------------------------------------------
# Film
# ---------------------------------------------------------------------------


def load_film(path=FILM_PATH):
    with open(path) as f:
        return json.load(f)


def expand(text, film):
    """Substitutes {VOCAB} placeholders; fails loudly on an unknown one."""

    def sub(m):
        key = m.group(1)
        if key not in film["vocab"]:
            raise KeyError(f"unknown placeholder {{{key}}}")
        return film["vocab"][key]

    return re.sub(r"\{([A-Z_]+)\}", sub, text)


def shots(film):
    for beat in film["beats"]:
        for shot in beat["shots"]:
            yield beat, shot


def shot_prompt(film, shot):
    return f"{film['style_prefix']}\n\n{expand(shot['prompt'], film)}"


def sheet_prompt(film, char_id):
    return f"{film['style_prefix']}\n\n{expand(film['characters'][char_id]['sheet_prompt'], film)}"


def motion_prompt(film, shot):
    action = expand(shot["motion"], film)
    return (
        f"Stop-motion claymation, handmade plasticine miniature, subtle frame-to-frame "
        f"jitter. {action[0].upper()}{action[1:]}. Keep every character's face, "
        f"proportions and costume exactly as in the image."
    )


# ---------------------------------------------------------------------------
# Cache + approvals
# ---------------------------------------------------------------------------


def cache_key(*parts):
    h = hashlib.sha256()
    for p in parts:
        h.update(json.dumps(p, sort_keys=True).encode())
        h.update(b"\0")
    return h.hexdigest()[:16]


def stage_dir(*names):
    d = BUILD.joinpath(*names)
    d.mkdir(parents=True, exist_ok=True)
    return d


def read_meta(path):
    p = Path(str(path) + ".json")
    return json.loads(p.read_text()) if p.exists() else None


def write_meta(path, meta):
    Path(str(path) + ".json").write_text(json.dumps(meta, indent=2))


def approved(kind, ident):
    """The take a human picked for a sheet or keyframe, or None.

    Approval is a symlink build/<kind>/<id>/approved.<ext> made by
    `run.py approve`. With one take, the first take counts as approved
    only when --auto-approve is passed.
    """
    d = BUILD / kind / ident
    for p in sorted(d.glob("approved.*")) if d.exists() else []:
        if not p.name.endswith(".json"):
            return p.resolve()
    return None


def takes(kind, ident):
    d = BUILD / kind / ident
    if not d.exists():
        return []
    return sorted(
        p for p in d.iterdir() if p.name.startswith("take") and not p.name.endswith(".json")
    )


def approve(kind, ident, take_no):
    d = BUILD / kind / ident
    matches = [p for p in takes(kind, ident) if p.stem.startswith(f"take{take_no}-")]
    if not matches:
        raise SystemExit(f"no take {take_no} for {kind}/{ident}; have: {[p.name for p in takes(kind, ident)]}")
    src = matches[0]
    for old in d.glob("approved.*"):
        old.unlink()
    link = d / f"approved{src.suffix}"
    link.symlink_to(src.name)
    meta = read_meta(src)
    if meta:
        write_meta(link, meta)
    log(f"approved {kind}/{ident} -> {src.name}")


# ---------------------------------------------------------------------------
# Budget ledger
# ---------------------------------------------------------------------------

LEDGER = BUILD / "ledger.json"


def ledger():
    if LEDGER.exists():
        return json.loads(LEDGER.read_text())
    return {"spent": 0.0, "entries": []}


def spent():
    return ledger()["spent"]


def record_spend(stage, what, usd):
    BUILD.mkdir(exist_ok=True)
    led = ledger()
    led["spent"] = round(led["spent"] + usd, 4)
    led["entries"].append({"t": time.strftime("%Y-%m-%dT%H:%M:%S"), "stage": stage, "what": what, "usd": usd})
    LEDGER.write_text(json.dumps(led, indent=2))


def check_budget(stage, estimate, budget, dry_run):
    already = spent()
    log(f"[{stage}] estimated new spend ${estimate:.2f}; already spent ${already:.2f}; budget ${budget:.2f}")
    if already + estimate > budget + 1e-9:
        msg = f"[{stage}] over budget: ${already:.2f} + ${estimate:.2f} > ${budget:.2f}"
        if dry_run:
            log(msg + " (dry run, continuing)")
        else:
            raise BudgetExceeded(msg + ". Raise --budget or trim the work.")


def require_key(name, dry_run):
    val = os.environ.get(name)
    if val or dry_run:
        return val
    raise SystemExit(f"{name} is not set; refusing to call a paid API. Use --dry-run to preview.")


# ---------------------------------------------------------------------------
# HTTP
# ---------------------------------------------------------------------------


def http_json(url, payload=None, headers=None, method=None, timeout=120):
    data = json.dumps(payload).encode() if payload is not None else None
    req = urllib.request.Request(url, data=data, method=method or ("POST" if data else "GET"))
    req.add_header("Content-Type", "application/json")
    for k, v in (headers or {}).items():
        req.add_header(k, v)
    try:
        with urllib.request.urlopen(req, timeout=timeout) as r:
            return json.loads(r.read())
    except urllib.error.HTTPError as e:
        raise SystemExit(f"HTTP {e.code} from {url}: {e.read()[:500].decode(errors='replace')}")


def download(url, dest):
    with urllib.request.urlopen(url, timeout=300) as r, open(dest, "wb") as f:
        f.write(r.read())
    return dest


def data_uri(path, mime=None):
    path = Path(path)
    mime = mime or {
        ".png": "image/png",
        ".jpg": "image/jpeg",
        ".jpeg": "image/jpeg",
        ".webp": "image/webp",
        ".mp3": "audio/mpeg",
        ".wav": "audio/wav",
    }[path.suffix.lower()]
    return f"data:{mime};base64,{base64.b64encode(path.read_bytes()).decode()}"


def remote_or_inline(path):
    """The fal-hosted URL a file came from, if we still have it; else a data URI."""
    meta = read_meta(path) or {}
    return meta.get("remote_url") or data_uri(path)


def fal_run(model, payload, poll_every=3.0, timeout=900):
    """Runs a model through fal's queue API and returns its result JSON."""
    key = os.environ["FAL_KEY"]
    headers = {"Authorization": f"Key {key}"}
    sub = http_json(f"https://queue.fal.run/{model}", payload, headers)
    status_url, response_url = sub["status_url"], sub["response_url"]
    t0 = time.time()
    while True:
        st = http_json(status_url, headers=headers)
        if st.get("status") == "COMPLETED":
            break
        if time.time() - t0 > timeout:
            raise SystemExit(f"fal {model} timed out after {timeout}s (request {sub.get('request_id')})")
        time.sleep(poll_every)
    return http_json(response_url, headers=headers)


# ---------------------------------------------------------------------------
# ffmpeg helpers
# ---------------------------------------------------------------------------


def run(cmd, capture=False):
    if capture:
        return subprocess.run(cmd, check=True, capture_output=True, text=True)
    subprocess.run(cmd, check=True)


def ffprobe_duration(path):
    out = run(
        ["ffprobe", "-v", "error", "-show_entries", "format=duration", "-of", "csv=p=0", str(path)],
        capture=True,
    ).stdout.strip()
    return float(out)


def loudness(path, start=None, end=None):
    """Integrated loudness (LUFS) of a file, optionally of a time window."""
    cmd = ["ffmpeg", "-hide_banner", "-nostats"]
    if start is not None:
        cmd += ["-ss", f"{start:.3f}"]
    if end is not None:
        cmd += ["-to", f"{end:.3f}"]
    cmd += ["-i", str(path), "-af", "ebur128=framelog=quiet", "-f", "null", "-"]
    err = subprocess.run(cmd, capture_output=True, text=True).stderr
    m = re.findall(r"I:\s+(-?[\d.]+|-inf) LUFS", err)
    if not m:
        return float("-inf")
    return float(m[-1]) if m[-1] != "-inf" else float("-inf")


def add_common_args(p, paid=True):
    p.add_argument("--film", default=str(FILM_PATH))
    if paid:
        p.add_argument("--dry-run", action="store_true", help="print prompts and cost, call nothing")
        p.add_argument("--budget", type=float, default=DEFAULT_BUDGET, help="cap for the whole film, USD")


def wrap(text, width=100, indent="    "):
    import textwrap

    return "\n".join(textwrap.fill(line, width, initial_indent=indent, subsequent_indent=indent) for line in text.split("\n"))


def die(msg):
    print(msg, file=sys.stderr)
    sys.exit(1)
