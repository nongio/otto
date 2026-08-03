#!/usr/bin/env bash
# Grab one frame straight off a virtual output's PipeWire node and write it as
# a PNG — the ground truth for "what is Otto actually putting in the stream?",
# with no RDP client, encoder, or bridge in the way.
#
#   ./capture-virtual.sh [output-name]     # default: virtual-1
#
# Run it while Otto is up (any VT, but Otto only renders on the ACTIVE VT).
# Writes /tmp/vout-<output>.png.

set -u
OUTPUT="${1:-virtual-1}"
OUT_PNG="/tmp/vout-${OUTPUT}.png"

# Find the node Otto tagged with this output name (see pipewire_stream.rs).
NODE=$(pw-dump | python3 -c '
import json, sys
name = sys.argv[1]
for obj in json.load(sys.stdin):
    props = (obj.get("info") or {}).get("props") or {}
    if props.get("otto.output.name") == name:
        print(obj["id"])
        break
' "$OUTPUT")

if [ -z "${NODE:-}" ]; then
  echo "no PipeWire node tagged otto.output.name=$OUTPUT"
  echo "(is the virtual output configured, and is this Otto build recent enough to tag its node?)"
  echo "nodes Otto is publishing:"
  pw-dump | grep -a "otto" | head
  exit 1
fi

echo "capturing one frame from node $NODE ($OUTPUT) -> $OUT_PNG"
# Linear dmabufs only — same negotiation the bridge does.
timeout 15 gst-launch-1.0 -q \
  pipewiresrc path="$NODE" num-buffers=5 \
  ! videoconvert \
  ! pngenc snapshot=true \
  ! filesink location="$OUT_PNG" \
  && echo "wrote $OUT_PNG" || echo "capture failed (is anything else already consuming the node?)"
ls -la "$OUT_PNG" 2>/dev/null
