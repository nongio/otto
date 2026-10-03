"""Stage 5b: cut the film to the narration, add captions, music and loudness.

Timeline: each beat lasts as long as its narration plus a short gap (the
last beat plus a short tail); its shots share that time in proportion to
their `duration`. With --no-voice, beats are timed at a fixed speaking rate
and the film is captions only.

Each shot uses, in order: the approved clip, take 1 of its clip, the approved
keyframe as a slow push-in, or a labelled placeholder card. So the edit runs
at any point in the pipeline and an animatic costs nothing.

Writes build/out/otto-clay.mp4, build/edit/timeline.json, and the
voice/music stems the QA stage measures.
"""

import json

from . import common as c
from . import motion, music

W, H, FPS = 1080, 1920, 24
OUT = c.BUILD / "out" / "otto-clay.mp4"


def fmt_ass_time(t):
    cs = int(round(t * 100))
    h, cs = divmod(cs, 360000)
    m, cs = divmod(cs, 6000)
    s, cs = divmod(cs, 100)
    return f"{h}:{m:02d}:{s:02d}.{cs:02d}"


def beat_speech(film, beat, no_voice, voice_dir="voice"):
    mp3 = c.BUILD / voice_dir / f"{beat['id']}.mp3"
    if not no_voice and mp3.exists():
        return c.ffprobe_duration(mp3), mp3
    return len(beat["vo"].split()) / film["edit"]["no_voice_words_per_second"], None


def build_timeline(film, no_voice, voice_dir="voice"):
    ed = film["edit"]
    t = 0.0
    beats, shots, captions = [], [], []
    missing_voice = []
    for i, beat in enumerate(film["beats"]):
        speech, mp3 = beat_speech(film, beat, no_voice, voice_dir)
        if not no_voice and mp3 is None:
            missing_voice.append(beat["id"])
        last = i == len(film["beats"]) - 1
        length = speech + (ed["tail_after_last_word"] if last else ed["gap_between_beats"])
        beats.append({"id": beat["id"], "start": t, "end": t + length, "speech_end": t + speech,
                      "words": len(beat["vo"].split()), "voice": str(mp3) if mp3 else None,
                      "vo": beat["vo"], "endcard": beat.get("endcard")})
        weights = [s["duration"] for s in beat["shots"]]
        st = t
        for s, w in zip(beat["shots"], weights):
            d = length * w / sum(weights)
            shots.append({"id": s["id"], "beat": beat["id"], "start": st, "end": st + d, "dur": d})
            st += d
        caps = beat.get("captions") or []
        cw = [len(x.split()) for x in caps]
        ct = t
        for k, (cap, w) in enumerate(zip(caps, cw)):
            d = speech * w / sum(cw)
            end = t + length if k == len(caps) - 1 else ct + d
            captions.append({"text": cap, "start": ct, "end": end})
            ct += d
        t += length
    return {"total": t, "beats": beats, "shots": shots, "captions": captions,
            "no_voice": no_voice, "missing_voice": missing_voice}


def ass_file(film, tl, path):
    font = film["edit"].get("font", "Inter")
    lines = [
        "[Script Info]", "ScriptType: v4.00+", f"PlayResX: {W}", f"PlayResY: {H}", "WrapStyle: 0", "",
        "[V4+ Styles]",
        "Format: Name, Fontname, Fontsize, PrimaryColour, SecondaryColour, OutlineColour, BackColour, Bold, Italic, "
        "Underline, StrikeOut, ScaleX, ScaleY, Spacing, Angle, BorderStyle, Outline, Shadow, Alignment, MarginL, "
        "MarginR, MarginV, Encoding",
        # Captions sit above the lower UI band of Shorts/Reels/TikTok.
        f"Style: Cap,{font},92,&H00FFFFFF,&H00FFFFFF,&H00301A10,&H64000000,1,0,0,0,100,100,0,0,1,7,3,2,90,90,640,1",
        f"Style: Title,{font},170,&H00FFFFFF,&H00FFFFFF,&H00301A10,&H64000000,1,0,0,0,100,100,0,0,1,8,4,8,80,80,330,1",
        f"Style: Tag,{font},70,&H00FFFFFF,&H00FFFFFF,&H00301A10,&H64000000,1,0,0,0,100,100,0,0,1,5,3,8,110,110,560,1",
        f"Style: Url,{font},56,&H00D8F4FF,&H00FFFFFF,&H00301A10,&H64000000,0,0,0,0,100,100,0,0,1,4,2,8,80,80,780,1",
        "", "[Events]", "Format: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text",
    ]
    for cap in tl["captions"]:
        pop = r"{\fad(60,60)\t(0,90,\fscx108\fscy108)\t(90,180,\fscx100\fscy100)}"
        lines.append(f"Dialogue: 0,{fmt_ass_time(cap['start'])},{fmt_ass_time(cap['end'])},Cap,,0,0,0,,{pop}{cap['text']}")
    for b in tl["beats"]:
        if not b["endcard"]:
            continue
        title, tag, url = b["endcard"]["lines"]
        s, e = fmt_ass_time(b["start"] + 0.15), fmt_ass_time(tl["total"])
        lines.append(f"Dialogue: 1,{s},{e},Title,,0,0,0,,{{\\fad(200,0)}}{title}")
        s2 = fmt_ass_time(b["start"] + 0.6)
        lines.append(f"Dialogue: 1,{s2},{e},Tag,,0,0,0,,{{\\fad(250,0)}}{tag}")
        lines.append(f"Dialogue: 1,{s2},{e},Url,,0,0,0,,{{\\fad(250,0)}}{url}")
    path.write_text("\n".join(lines) + "\n")


def shot_source(film, shot_id):
    clip = c.approved("clips", shot_id)
    if not clip:
        for j in motion.jobs(film, only=[shot_id]):
            if j["path"].exists():
                clip = j["path"]
                break
    if clip:
        return "clip", clip
    kf = c.approved("keyframes", shot_id) or next(iter(c.takes("keyframes", shot_id)), None)
    if kf:
        return "keyframe", kf
    return "placeholder", None


def render_segment(film, shot, dest):
    kind, src = shot_source(film, shot["id"])
    d = shot["dur"]
    fit = f"scale={W}:{H}:force_original_aspect_ratio=increase,crop={W}:{H},setsar=1"
    if kind == "clip":
        cmd = ["-i", str(src), "-vf", f"{fit},fps={FPS},tpad=stop_mode=clone:stop_duration=10"]
    elif kind == "keyframe":
        frames = int(d * FPS) + 2
        cmd = ["-loop", "1", "-i", str(src), "-vf",
               f"scale={W * 2}:{H * 2}:force_original_aspect_ratio=increase,crop={W * 2}:{H * 2},"
               f"zoompan=z='1+0.04*on/{frames}':x='iw/2-iw/zoom/2':y='ih/2-ih/zoom/2':d={frames}:s={W}x{H}:fps={FPS},setsar=1"]
    else:
        beat = next(b for b in film["beats"] if any(s["id"] == shot["id"] for s in b["shots"]))
        hue = (int(shot["id"][1:]) * 47) % 360
        label = f"{shot['id']}  placeholder"
        cmd = ["-f", "lavfi", "-i", f"color=c=0x2b2420:s={W}x{H}:r={FPS}", "-vf",
               f"hue=h={hue},drawtext=text='{label}':fontcolor=0xf2e6d8:fontsize=64:x=(w-tw)/2:y=h*0.80,"
               f"drawtext=text='{beat['id']}':fontcolor=0xb8a898:fontsize=44:x=(w-tw)/2:y=h*0.80+90"]
    c.run(["ffmpeg", "-y", "-v", "error", *cmd, "-t", f"{d:.3f}", "-an", "-r", str(FPS),
           "-c:v", "libx264", "-preset", "veryfast", "-crf", "16", "-pix_fmt", "yuv420p", str(dest)])
    return kind


def voice_stem(tl, dest):
    total = tl["total"]
    inputs, filters, labels = [], [], []
    for b in tl["beats"]:
        if not b["voice"]:
            continue
        idx = len(inputs) // 2
        inputs += ["-i", b["voice"]]
        ms = int(round(b["start"] * 1000))
        filters.append(f"[{idx}:a]aresample=44100,aformat=channel_layouts=stereo,adelay={ms}|{ms}[v{idx}]")
        labels.append(f"[v{idx}]")
    if not labels:
        c.run(["ffmpeg", "-y", "-v", "error", "-f", "lavfi", "-i", "anullsrc=r=44100:cl=stereo",
               "-t", f"{total:.3f}", str(dest)])
        return False
    filters.append(f"{''.join(labels)}amix=inputs={len(labels)}:normalize=0,apad,atrim=0:{total:.3f}[out]")
    c.run(["ffmpeg", "-y", "-v", "error", *inputs, "-filter_complex", ";".join(filters), "-map", "[out]", str(dest)])
    return True


def loudnorm(src, dest, target):
    args = f"I={target}:TP=-1.5:LRA=11"
    err = c.run(["ffmpeg", "-hide_banner", "-nostats", "-i", str(src), "-af", f"loudnorm={args}:print_format=json",
                 "-f", "null", "-"], capture=True).stderr
    m = json.loads(err[err.rindex("{"):err.rindex("}") + 1])
    second = (f"loudnorm={args}:measured_I={m['input_i']}:measured_TP={m['input_tp']}:measured_LRA={m['input_lra']}"
              f":measured_thresh={m['input_thresh']}:offset={m['target_offset']}:linear=true")
    c.run(["ffmpeg", "-y", "-v", "error", "-i", str(src), "-af", second, "-ar", "44100", str(dest)])


def run(args, film):
    work = c.stage_dir("edit")
    voice_dir = "voice-scratch" if args.scratch else "voice"
    tl = build_timeline(film, args.no_voice, voice_dir)
    tl["scratch_voice"] = args.scratch
    if tl["missing_voice"]:
        c.log(f"[edit] no narration yet for {tl['missing_voice']}: timing them at the no-voice rate")
    c.log(f"[edit] {len(tl['shots'])} shots, {tl['total']:.2f}s")

    kinds = {}
    seg_list = work / "segments.txt"
    with open(seg_list, "w") as f:
        for shot in tl["shots"]:
            seg = work / f"seg_{shot['id']}.mp4"
            kinds[shot["id"]] = shot["source"] = render_segment(film, shot, seg)
            f.write(f"file '{seg.name}'\n")
    c.log("  sources: " + ", ".join(f"{k}={v}" for k, v in kinds.items()))
    video = work / "video.mp4"
    c.run(["ffmpeg", "-y", "-v", "error", "-f", "concat", "-safe", "0", "-i", str(seg_list), "-c", "copy", str(video)])

    ass = work / "captions.ass"
    ass_file(film, tl, ass)

    voice = work / "voice_stem.wav"
    has_voice = voice_stem(tl, voice)
    bed = music.run(args, film, seconds=tl["total"] + 2)
    target = film["edit"]["target_lufs"]
    music_lufs = c.loudness(bed, 0, tl["total"])
    if has_voice:
        voice_lufs = c.loudness(voice)
        music_gain = (voice_lufs - film["edit"]["voice_lufs_offset_db"]) - music_lufs
    else:
        music_gain = target - music_lufs
    music_stem = work / "music_stem.wav"
    c.run(["ffmpeg", "-y", "-v", "error", "-i", str(bed), "-af",
           f"atrim=0:{tl['total']:.3f},volume={music_gain:.2f}dB,afade=t=out:st={max(0, tl['total'] - 1.2):.3f}:d=1.2",
           "-ar", "44100", "-ac", "2", str(music_stem)])
    premix = work / "premix.wav"
    c.run(["ffmpeg", "-y", "-v", "error", "-i", str(voice), "-i", str(music_stem), "-filter_complex",
           "[0:a][1:a]amix=inputs=2:normalize=0:duration=longest[a]", "-map", "[a]", str(premix)])
    mix = work / "mix.wav"
    loudnorm(premix, mix, target)

    OUT.parent.mkdir(parents=True, exist_ok=True)
    c.run(["ffmpeg", "-y", "-v", "error", "-i", str(video), "-i", str(mix), "-vf", f"ass={ass}",
           "-map", "0:v", "-map", "1:a", "-t", f"{tl['total']:.3f}", "-c:v", "libx264", "-preset", "medium",
           "-crf", "18", "-pix_fmt", "yuv420p", "-c:a", "aac", "-b:a", "192k", "-movflags", "+faststart", str(OUT)])
    tl["music_gain_db"] = music_gain
    tl["has_voice"] = has_voice
    (work / "timeline.json").write_text(json.dumps(tl, indent=2))
    c.log(f"[edit] -> {OUT.relative_to(c.ROOT)} ({c.ffprobe_duration(OUT):.2f}s)")
