#!/usr/bin/env bash
# Builds everything film.html pulls in at render time into build/:
#   build/img/<name>.jpg        screenshots, 1400 px wide
#   build/seq/<clip>/NNNN.jpg   frames of the recordings in assets/, 24 fps
#   build/fonts.css             Google Fonts, latin subsets, inlined as data: URIs
# Run `npm install` first; ffmpeg comes from ffmpeg-static.
set -euo pipefail
cd "$(dirname "$0")"
ROOT=../..
FF=node_modules/ffmpeg-static/ffmpeg
mkdir -p build/img build/seq

for f in assets/press/01-desktop-hero.jpg assets/press/03-expose.jpg \
         docs/user/images/tiling.jpg docs/user/images/rice-crate-digger.jpg \
         docs/user/images/rice-deep-field.jpg docs/user/images/rice-section-9.jpg \
         docs/user/images/rice-yorha-bone.jpg docs/user/images/rice-shibuya-heist.jpg \
         docs/user/images/rice-beton-brut.jpg; do
  name=$(basename "${f%.*}")
  "$FF" -loglevel error -y -i "$ROOT/$f" -vf scale=1400:-2 -q:v 4 "build/img/$name.jpg"
done

# clip name, source recording, start (s), length (s). film.html's data-frames
# must match the frame count printed here.
clip() {
  mkdir -p "build/seq/$1"
  "$FF" -loglevel error -y -ss "$3" -t "$4" -i "$ROOT/assets/$2" -vf fps=24 -q:v 3 "build/seq/$1/%04d.jpg"
  echo "$1: $(ls "build/seq/$1" | wc -l) frames"
}
clip minimize   2-dock-minimize-windows.mp4   0   7.2
clip expose     6-expose-windows.mp4          0.3 4.6
clip workspaces 3-move-windows-workspaces.mp4 3.2 5.0
clip switcher   7-app-switcher.mp4            0.6 4.4

# Chromium in the render cannot always reach fonts.googleapis.com (proxies,
# offline CI), so the fonts are fetched once here and inlined.
UA="Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 Chrome/140 Safari/537.36"
curl -fsS -A "$UA" "https://fonts.googleapis.com/css2?family=Inter+Tight:wght@500;700;800&family=Instrument+Serif:ital@0;1&family=JetBrains+Mono:wght@400;600&display=block" -o build/fonts.src.css
python3 - <<'PY'
import base64, re, subprocess
css = open("build/fonts.src.css").read()
out = []
for subset, block in re.findall(r"/\* ([\w-]+) \*/\s*(@font-face \{.*?\})", css, re.S):
    if subset not in ("latin", "latin-ext"):
        continue
    url = re.search(r"url\((.*?)\)", block).group(1)
    data = subprocess.run(["curl", "-fsS", url], capture_output=True, check=True).stdout
    out.append(block.replace(url, "data:font/woff2;base64," + base64.b64encode(data).decode()))
open("build/fonts.css", "w").write("\n".join(out))
print(f"fonts: {len(out)} faces")
PY
