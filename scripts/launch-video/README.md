# Launch video

A 45-second, 1920×1080, 30 fps launch video for Otto, written as one HTML page
(`film.html`) and rendered frame by frame in headless Chromium, then encoded
with ffmpeg. There is no video model and no editor: every frame is a
deterministic function of film time. The approach follows
[launchvideo.io](https://launchvideo.io) (source:
[diggerhq/shipvideo](https://github.com/diggerhq/shipvideo)); this kit keeps its
pipeline but adds Otto's real screenshots and screen recordings, which
launchvideo.io's hosted agent does not allow.

The current cut has been approved by the maintainer. Treat it as the baseline:
change it on request, don't redesign it.

## Build it

```sh
cd scripts/launch-video
npm install
bash prepare.sh                      # screenshots, clip frames, inlined fonts -> build/
node render.mjs stills out 10 26.5   # JPEG stills at those seconds -> out/t10.jpg ...
node render.mjs video                # full render -> out/otto-launch.mp4 (~1.5 min)
```

If Playwright cannot find Chromium, point `PLAYWRIGHT_CHROMIUM` at a
`chromium` or `headless_shell` binary (in Claude Code cloud sessions:
`/opt/pw-browsers/chromium_headless_shell-1194/chrome-linux/headless_shell`).
Do not run `playwright install` there.

`build/` and `out/` are git-ignored. Don't commit the MP4 (about 12 MB); attach
it to a release or hand it to the maintainer directly.

## How the film works

- **Timeline.** `B` in the `<script>` lists the 12 beats as `[id, start, end]`
  in seconds; `DUR` is the total. Exactly one beat `div` is visible at a
  time. `frame(ms)` computes each beat's local time `L` and positions its
  elements from `L` (the `switch (active)` block). To retime, edit `B` and
  `DUR`, then pass the new duration to `render.mjs video`.
- **Clock.** `render.mjs` replaces `requestAnimationFrame` and
  `performance.now` before the page loads, and `__seek(ms)` runs one frame at
  that time. Animate only from `L`/`T`: no CSS transitions, no timers, no
  `Date`, no `Math.random`. Anything else renders differently each run.
- **Screenshots** are `{{img:<name>}}` placeholders, inlined from
  `build/img/<name>.jpg`. Add one by adding its path to the loop in
  `prepare.sh`.
- **Recordings** are `<img class="clip">` elements, not `<video>`: a `<video>`
  seek can complete before Chromium paints the frame, which gave blank frames.
  `__syncVideos()` picks the frame for the current time from
  `data-beat` (which beat it plays in), `data-delay` (seconds into the beat
  before it starts), `data-rate` (playback speed) and `data-frames` (the count
  `prepare.sh` prints), and waits for it to decode. `render.mjs` serves
  `http://clips.local/<clip>/NNNN.jpg` from `build/seq/`. To change a clip's
  in/out points, edit its `clip` line in `prepare.sh` and update
  `data-frames`.
- **Fonts** are Inter Tight, Instrument Serif and JetBrains Mono, inlined by
  `prepare.sh`. The page does no network fetches at render time.

## Beats

| # | Time (s) | On screen | Source |
|---|---|---|---|
| 1 | 0–2.7 | "Smooth." / "Thoughtful." / "Yours." | — |
| 2 | 2.7–5.9 | "A Wayland desktop *that feels like someone cared.*" | README tagline |
| 3 | 5.9–8.9 | "otto", "a compositor built from scratch in Rust" | — |
| 4 | 8.9–14.2 | 01 The Dock: "A Dock that's a real task manager." | `2-dock-minimize-windows.mp4` at 1.35× |
| 5 | 14.2–18.4 | 02 Exposé: "Every window. Live." + `PageUp` | `6-expose-windows.mp4` |
| 6 | 18.4–22.8 | 03 Workspaces: "Drag it to another desk." | `3-move-windows-workspaces.mp4` at 1.1× |
| 7 | 22.8–26.6 | 04 App switcher: "Apps, not just windows." + `Ctrl Tab` | `7-app-switcher.mp4` |
| 8 | 26.6–30 | 05 Tiling: "Tiling *or stacking.*" | `docs/user/images/tiling.jpg` |
| 9 | 30–33.4 | Six themed desktops: "One config file. *Any look.*" | `docs/user/images/rice-*.jpg` |
| 10 | 33.4–37 | 06 Rendering: display planes drawn in 3D, "Skia + hardware planes." | — |
| 11 | 37–40 | Screen sharing, Remote desktop, Lock & login, Files, Live settings, Launcher, 11 languages | README feature list |
| 12 | 40–45 | End card: tagline, `$ otto --winit`, `github.com/nongio/otto` | — |

Every claim comes from the root `README.md`. Keep it that way: don't invent
numbers, and re-check the feature list against the README when it changes.
"11 languages" counts the README's 12 locales with English GB/US as one.

## Checking your work

Render stills at the moments you changed, plus one on each side of every beat
boundary you touched, and look at them before a full render. A contact sheet
is quicker than opening files one by one:

```sh
node render.mjs video out/check.mp4 45
node_modules/ffmpeg-static/ffmpeg -y -i out/check.mp4 -vf "fps=2/3,scale=384:-1,tile=6x5" -frames:v 1 out/sheet.jpg
```

Look for: text left over from the previous beat, a beat with nothing on
screen, a blank clip frame, captions that overlap a screenshot or the corner
labels, and text running off the 1920 px frame. `render.mjs` exits non-zero
on any page error.

## Open items

These are known and waiting on the maintainer. Don't fix them unasked.

- **Third-party footage.** The minimize and workspace recordings show a Daft
  Punk music video playing in a window. That is fine in the README, but a
  public upload could be flagged or taken down. Re-recording those two clips
  with neutral content would fix it; the Exposé and app-switcher clips are clean.
- **Sped-up clips.** Minimize runs at 1.35× and workspaces at 1.1× to fit
  their beats. Lengthening those beats would allow real-time playback.
- **No audio.** A music bed would be muxed in with ffmpeg after rendering.
- **Unused recordings:** `1-dock-taskmanager.mp4` (Dock magnification),
  `4-dock-navigate-apps.mp4`, `5-workspace-selector.mp4`.
