# otto-clay

An experiment: a 30 to 45 second vertical claymation short about Otto, made by
a small script pipeline. The approach follows
[Twelve AI clay films](https://getsweat.ai/blog/twelve-ai-clay-films/):
the script is data, there's one reference sheet per character attached to every
shot, stills come first and motion second, the cut follows the narration, and
a QA pass measures the rules the article found mattered.

Nothing here touches the rest of the repo. Generated assets go to `build/`,
which git ignores.

## The character

**Moss** is a small, plump, olive-green clay toad. He wears round brass wire
glasses and a lumpy mustard knitted cardigan with three mismatched felt
buttons. He's fussy and easily cross, and he presses buttons with his sticky
tongue. His desk is a tiny balsa-wood one by a window at dusk. He sits on a
stack of matchboxes, with a chunky cream clay monitor, a knitted keyboard and
a felt touchpad.

**Logline:** Moss is buried under forty overlapping windows until Otto turns
his Linux desktop into something he can't stop playing with, and he ends up
dancing to his song on the island.

## The story, beat by beat

Timings are from the free scratch-voice animatic (34 s, 13 shots). The real
timings come from the ElevenLabs narration.

| Beat | Narration | Shots | Otto feature |
|---|---|---|---|
| b01 (0.0 s) | Moss slams his keyboard, and forty windows pile up on top of each other. Again. | s01 close-up: hands raised over the keyboard, then slamming. s02 over the shoulder: the monitor buried under a crooked pile of windows | (the mess) |
| b02 (4.5 s) | So he installs Otto, a free, open-source desktop for Linux, built in Rust. | s03: he presses a key, the screen turns calm and tidy, and he grins | Otto |
| b03 (8.7 s) | One lick of the little dot, and the mess funnels into the Dock like a genie. | s04: his tongue taps the minimise dot. s05: a window genie-funnels into the Dock | Minimise, genie |
| b04 (12.1 s) | Lost something? Three fingers up, and every single window spreads out. | s06: three clay fingers swipe up on the felt touchpad. s07: the Exposé grid with the workspace strip on top | Exposé |
| b05 (16.0 s) | Swipe sideways with three fingers, and a fresh workspace slides right in. | s08: the screen is mid-slide between two workspaces while Moss leans with it | Workspace swipe |
| b06 (19.8 s) | Still crowded? Tile them into neat little squares, nothing overlapping. | s09: tiled windows, and he pushes his glasses up and nods | Tiling |
| b07 (23.7 s) | Then his favourite song comes on, and the island up top starts to dance. | s10: the island pill with album art and bouncing bars. s11: Moss bopping with his eyes closed | Island music |
| b08 (27.6 s) | So does Moss. Honestly, the work can wait a minute or two. | s12 wide shot: he dances on the matchboxes and snaps up a fly with his tongue | |
| b09 (30.9 s) | Otto. A Wayland desktop that's a pleasure to use. | s13: he sips tea under an empty wall. The end card text is drawn by ffmpeg: "Otto", the tagline, nongio.github.io/otto | End card (3.1 s) |

These are the article's rules, applied to this script:

- The opening is a physical action (the slam), with no title and no logo.
- The second line introduces something new: Otto.
- Captions are 2 to 4 words.
- The ending is 6 s or less.
- The video stops 0.75 s or less after the last word.
- There are no gaps longer than 0.75 s.
- Speech is 3 words/s or faster.
- Voice sits 13 to 26 dB over the music (the target is 18).
- Loudness is −15 to −13 LUFS (the target is −14).

The median shot is about 2.2 s. The article's ceiling is 2.6 s.

All the features shown are real: minimise with the genie animation
(`docs/user/window-management.md`), Exposé with a three-finger swipe up
(`expose-and-switcher.md`), three-finger horizontal workspace swipes
(`workspaces.md`), tiling (`tiling.md`), and the island music activity. The
island music activity is in `components/otto-islands/src/music.rs`.
`docs/user/dynamic-island.md` still lists MPRIS as unsupported; that line is
out of date.

## Files

- `film.json`: the whole film as data. It holds the 121-word clay
  `style_prefix` and a `vocab` of fixed descriptions (`{MOSS}`, `{DESK}`,
  `{OTTO_SCREEN}`) that are substituted into every prompt so the character
  and set stay word-for-word identical. It also has the character sheet
  prompt, and the beats with `vo`, `captions`, and shots with `refs`,
  `prompt`, `motion` and `duration`, plus voice, music and edit settings.
  Scenes with no character say "no living thing". Anything that moves is
  already in the still. The image model never draws text; captions and the
  end card are drawn by ffmpeg.
- `run.py`: one command per stage. `Makefile` wraps it.
- `clay/`: one small module per stage. They use only the standard library,
  plus `ffmpeg`/`ffprobe`. `espeak-ng` is optional, for scratch narration.
- `dry-run.log`: output of `make plan`. It shows every prompt, fully
  expanded except the style prefix, which is printed once, and the estimated
  spend.

## Running it

You need Python 3.10+ and ffmpeg built with libass. You also need the keys
below, set as environment variables. Nothing reads keys from anywhere else.

```sh
export FAL_KEY=...              # fal.ai: images, video, (optional) Whisper
export ELEVENLABS_API_KEY=...   # ElevenLabs: narration (skip with --no-voice)
cd experiments/otto-clay
```

| Step | Command | What it does | Paid? |
|---|---|---|---|
| 0 | `make plan` | Lints the script, prints every prompt and the cost, calls nothing | no |
| 0b | `make animatic` | espeak-ng scratch voice, placeholder cards, music, captions, full QA. Checks the timing for free | no |
| 1 | `./run.py voice` | ElevenLabs narration per beat (`/with-timestamps`), trimmed to the spoken span | yes |
| 2 | `./run.py transcribe` | Whisper transcribes each clip back and fails on more than 10% word error | free locally* |
| 3 | `./run.py sheet --takes 3` | Moss reference sheet candidates (`fal-ai/nano-banana`) | yes |
| 3b | `./run.py approve sheet moss 2` | **you** pick the sheet | no |
| 4 | `./run.py keyframes --takes 2` | Stills with the approved sheet attached (`fal-ai/nano-banana/edit`); empty scenes use text-to-image | yes |
| 4b | `./run.py approve keyframe s01 1` | **you** pick a still per shot (13 picks) | no |
| 5 | `./run.py motion` | 5 s clips (`fal-ai/kling-video/v3/pro/image-to-video`, audio off) | yes |
| 6 | `./run.py edit` | Cut to the narration, ASS captions and end card, synthesised music bed, two-pass loudnorm. Writes `build/out/otto-clay.mp4` | no |
| 7 | `./run.py qa` | Contact sheets (8 frames per shot of the final file, 4 fps per clip) and the measured checks. Writes `build/qa/` | no |

\* `transcribe` uses `faster-whisper` or `openai-whisper` if either is
installed. Otherwise it uses `fal-ai/whisper`, at about a cent per beat.

The common flags work on every stage:

- `--dry-run`
- `--budget N`: the default is 15, the cap for the whole film.
- `--only s04,s05`: redo just some shots.
- `--takes N`: more candidates.
- `--no-voice`: captions only, timed at 3.2 words/s.
- `--auto-approve`: take 1 counts as approved.

`./run.py all --auto-approve` runs every stage in order. `./run.py status`
shows what exists, what's approved and what was spent.

`edit` works at any point. For each shot it uses the approved clip, then take
1 of the clip, then the approved keyframe as a slow push-in, and finally a
labelled placeholder card. So you can watch the film take shape as you pay
for it.

### Money safety

- Every paid stage prints its estimate before doing anything. Then it checks
  the estimate plus what `build/ledger.json` says was already spent against
  `--budget`, and refuses if that would go over.
- `--dry-run` never calls anything.
- Without the key a stage needs, it refuses and exits.
- Every asset is cached under `build/` on a hash of exactly what produced it:
  model, prompt, attached sheet, take number. A rerun costs nothing, and
  changing one prompt only pays for that shot.
- Changing one beat's narration also re-renders its two neighbours. They are
  sent as context so the intonation stays continuous, at about $0.02 each.

## Cost estimate

These are list prices from October 2026: Nano Banana at $0.039 per image,
Kling 3 Pro at $0.112/s with audio off, and ElevenLabs at about $0.30 per 1k
characters. The ElevenLabs rate depends on your plan.

| Stage | Estimate | How |
|---|---|---|
| Narration | $0.19 | 622 characters |
| Whisper check | $0.00 | local; at most about $0.09 on fal |
| Character sheet | $0.12 | 3 takes |
| Keyframes | $1.01 | 13 shots x 2 takes |
| Motion | $7.28 | 13 clips x 5 s |
| Music, edit, QA | $0.00 | local |
| **First pass** | **$8.60** | |
| Retake headroom | $6.40 | within the $15 cap |

The article lost about 20% of its spend to rejected takes. The cheapest place
to catch a bad shot is the keyframe approval: one keyframe costs $0.04, one
clip costs $0.56. Regenerating every clip once more would cost $7.28 and would
not fit, so retake with `--only` and `--takes`.

## What needs a human

The scripts handle what can be measured. Taste stays with you:

1. **Character.** Is Moss the right star? The `MOSS` vocab line and the sheet
   prompt are the places to change him.
2. **Style approval.** Pick the reference sheet (`approve sheet`), then one
   keyframe per shot (`approve keyframe`). Look at
   `build/qa/contact_final.png` and the clip sheets for drift: a second toad,
   text on a screen, a melted face.
3. **Story and words.** The narration, the captions, and whether each Otto
   moment reads on screen. Run `make animatic` after any script edit.
4. **The voice.** `voice.voice_id` in `film.json` is a stock ElevenLabs voice.
   Pick one that sounds right.
5. **Music.** `clay/music.py` synthesises a cheerful plucked-string bed, so
   there's nothing to license. Swap in another royalty-free track if you like.
   The same track is the song playing on the island.
6. **Posting.** The article labels the films as AI-made on every platform.
