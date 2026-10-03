"""Stage 2a: narration per beat via ElevenLabs, with character timestamps.

Writes build/voice/<beat>.mp3 (trimmed to the spoken span) and
build/voice/<beat>.align.json. Cached on (text, voice, model, settings).
"""

import base64
import json

from . import common as c

API = "https://api.elevenlabs.io/v1/text-to-speech/{voice}/with-timestamps?output_format=mp3_44100_128"
PAD = 0.04  # seconds kept before the first and after the last character


def plan(film):
    v = film["voice"]
    out = []
    beats = film["beats"]
    for i, beat in enumerate(beats):
        body = {
            "text": beat["vo"],
            "model_id": v["model_id"],
            "voice_settings": v["settings"],
            # Context keeps the intonation continuous across beats.
            "previous_text": beats[i - 1]["vo"] if i else None,
            "next_text": beats[i + 1]["vo"] if i + 1 < len(beats) else None,
        }
        key = c.cache_key("elevenlabs", v["voice_id"], body)
        mp3 = c.BUILD / "voice" / f"{beat['id']}.mp3"
        meta = c.read_meta(mp3) or {}
        cached = mp3.exists() and meta.get("key") == key
        cost = 0 if cached else len(beat["vo"]) / 1000 * c.PRICES["elevenlabs_per_1k_chars"]
        out.append((beat, body, key, mp3, cached, cost))
    return out


def scratch(film):
    """Free placeholder narration from espeak-ng, for checking timing before paying."""
    import shutil

    tts = shutil.which("espeak-ng") or shutil.which("espeak")
    if not tts:
        raise SystemExit("[voice] --scratch needs espeak-ng")
    d = c.stage_dir("voice-scratch")
    trim = ("silenceremove=start_periods=1:start_threshold=-45dB,areverse,"
            "silenceremove=start_periods=1:start_threshold=-45dB,areverse")
    for beat in film["beats"]:
        wav, mp3 = d / f"{beat['id']}.wav", d / f"{beat['id']}.mp3"
        c.run([tts, "-v", "en-gb", "-s", "195", "-w", str(wav), beat["vo"]])
        c.run(["ffmpeg", "-y", "-v", "error", "-i", str(wav), "-af", trim, "-ar", "44100", str(mp3)])
        wav.unlink()
        c.log(f"  {beat['id']} scratch {c.ffprobe_duration(mp3):.2f}s: {beat['vo']}")


def run(args, film):
    if getattr(args, "scratch", False):
        c.log("[voice] --scratch: espeak-ng placeholder narration (free) in build/voice-scratch/")
        return scratch(film)
    if args.no_voice:
        c.log("[voice] --no-voice: skipping narration; the edit uses captions only")
        return
    items = plan(film)
    est = sum(i[5] for i in items)
    chars = sum(len(i[0]["vo"]) for i in items if not i[4])
    c.log(f"[voice] {len(items)} beats, {chars} new characters at ${c.PRICES['elevenlabs_per_1k_chars']}/1k chars")
    for beat, body, key, mp3, cached, cost in items:
        c.log(f"  {beat['id']} {'(cached)' if cached else f'${cost:.3f}'}: {beat['vo']}")
    c.check_budget("voice", est, args.budget, args.dry_run)
    if args.dry_run or est == 0:
        return
    api_key = c.require_key("ELEVENLABS_API_KEY", args.dry_run)
    c.stage_dir("voice")
    voice = film["voice"]["voice_id"]
    for beat, body, key, mp3, cached, cost in items:
        if cached:
            continue
        body = {k: v for k, v in body.items() if v is not None}
        res = c.http_json(API.format(voice=voice), body, {"xi-api-key": api_key, "Accept": "application/json"})
        c.record_spend("voice", beat["id"], cost)
        raw = mp3.with_suffix(".raw.mp3")
        raw.write_bytes(base64.b64decode(res["audio_base64"]))
        al = res.get("normalized_alignment") or res["alignment"]
        start = max(0.0, al["character_start_times_seconds"][0] - PAD)
        end = al["character_end_times_seconds"][-1] + PAD
        c.run(["ffmpeg", "-y", "-v", "error", "-i", str(raw), "-ss", f"{start:.3f}", "-to", f"{end:.3f}",
               "-c:a", "libmp3lame", "-b:a", "192k", str(mp3)])
        mp3.with_suffix(".align.json").write_text(json.dumps({"offset": start, "alignment": al}, indent=1))
        c.write_meta(mp3, {"key": key, "text": beat["vo"], "duration": c.ffprobe_duration(mp3)})
        c.log(f"  {beat['id']} -> {mp3.name} ({c.ffprobe_duration(mp3):.2f}s)")
