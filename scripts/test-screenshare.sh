#!/usr/bin/env bash
set -euo pipefail

SERVICE="org.otto.ScreenCast"
ROOT_PATH="/org/otto/ScreenCast"
ROOT_IFACE="org.otto.ScreenCast"
SESSION_IFACE="org.otto.ScreenCast.Session"

OUTPUT="${1:-eDP-1}"
OPEN_PIPEWIRE="${OPEN_PIPEWIRE:-0}"
PLAYER="${PLAYER:-gst}" # gst|ffplay|none
CLEANED_UP=0

require_cmd() {
    if ! command -v "$1" >/dev/null 2>&1; then
        echo "Missing required command: $1" >&2
        exit 1
    fi
}

extract_object_path() {
    awk -F'"' '{print $2}'
}

require_cmd busctl
require_cmd awk
require_cmd grep

echo "Listing outputs..."
OUTPUTS_RAW="$(busctl --user call "$SERVICE" "$ROOT_PATH" "$ROOT_IFACE" ListOutputs)"
echo "  $OUTPUTS_RAW"

if ! grep -q "\"$OUTPUT\"" <<<"$OUTPUTS_RAW"; then
    echo "Output '$OUTPUT' not found. Pick one from the list above." >&2
    exit 1
fi

echo "Creating session..."
SESSION_PATH="$(
    busctl --user call "$SERVICE" "$ROOT_PATH" "$ROOT_IFACE" CreateSession a{sv} 0 \
        | extract_object_path
)"
echo "  session: $SESSION_PATH"

cleanup() {
    if [[ "$CLEANED_UP" == "1" ]]; then
        return
    fi
    CLEANED_UP=1

    if [[ -n "${SESSION_PATH:-}" ]]; then
        echo
        echo "Stopping session..."
        busctl --user call "$SERVICE" "$SESSION_PATH" "$SESSION_IFACE" Stop >/dev/null 2>&1 || true
    fi
}
trap cleanup EXIT INT TERM

echo "Recording monitor '$OUTPUT'..."
STREAM_PATH="$(
    busctl --user call "$SERVICE" "$SESSION_PATH" "$SESSION_IFACE" RecordMonitor sa{sv} "$OUTPUT" 0 \
        | extract_object_path
)"
echo "  stream: $STREAM_PATH"

echo "Starting session..."
busctl --user call "$SERVICE" "$SESSION_PATH" "$SESSION_IFACE" Start
echo "Session started."

if [[ "$OPEN_PIPEWIRE" == "1" ]]; then
    echo "Opening PipeWire remote..."
    busctl --user call "$SERVICE" "$SESSION_PATH" "$SESSION_IFACE" OpenPipeWireRemote a{sv} 0
fi

echo "Reading stream metadata..."
NODE="$(
    busctl --user call "$SERVICE" "$STREAM_PATH" org.otto.ScreenCast.Stream PipeWireNode \
        | awk '{print $NF}'
)"
echo "  node-id: $NODE"
busctl --user call "$SERVICE" "$STREAM_PATH" org.otto.ScreenCast.Stream Metadata

case "$PLAYER" in
    none)
        echo "Press Ctrl+C to stop."
        while true; do
            sleep 1
        done
        ;;
    gst)
        require_cmd gst-launch-1.0
        for sink in waylandsink autovideosink ximagesink glimagesink; do
            echo "Opening stream with GStreamer (sink: $sink)..."
            set +e
            gst-launch-1.0 pipewiresrc path="$NODE" do-timestamp=true ! videoconvert ! queue ! "$sink" sync=false
            status=$?
            set -e
            if [[ "$status" -eq 0 ]]; then
                break
            fi
            if [[ "$status" -eq 130 ]]; then
                exit 130
            fi
            echo "Sink failed: $sink"
        done
        ;;
    ffplay)
        require_cmd ffplay
        echo "Opening stream with ffplay..."
        ffplay -f pipewire -i "$NODE"
        ;;
    *)
        echo "Invalid PLAYER value '$PLAYER'. Use one of: none, gst, ffplay" >&2
        exit 1
        ;;
esac
