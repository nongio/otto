#!/usr/bin/env bash
# Start Otto + the RDP bridge on this VT, with full logging for diagnosis.
#
#   ./run-rdp.sh            # serve the physical screen (eDP-1)
#   ./run-rdp.sh virtual    # serve the virtual output (virtual-1)
#
# Run this ON tty2: Otto only renders while its VT is active, so if you switch
# away the feed freezes (that is not a bug — libseat pauses the DRM session).
#
# Ctrl+C stops both. On exit it prints WHY Otto went away.
#
# NOTE: Super+Q and Ctrl+Alt+Backspace quit Otto instantly (hardcoded in
# process_keyboard_shortcut, checked before the config). Avoid them.

set -u
cd "$(dirname "$0")" || exit 1

MODE="${1:-physical}"
OTTO_LOG=/tmp/otto-user.log
RDP_LOG=/tmp/otto-rdp.log
PORT=3389

cleanup() {
  echo
  echo "stopping..."
  [ -n "${RDP_PID:-}" ] && kill "$RDP_PID" 2>/dev/null
  [ -n "${OTTO_PID:-}" ] && kill -INT "$OTTO_PID" 2>/dev/null
  exit 0
}
trap cleanup INT TERM

# ── clean slate ───────────────────────────────────────────────────────────
pkill -f 'release/otto-rdp' 2>/dev/null
pkill -f 'debug/otto-rdp' 2>/dev/null
pkill -f 'release/otto$' 2>/dev/null
sleep 1

# ── otto ──────────────────────────────────────────────────────────────────
echo "starting otto  -> $OTTO_LOG"
RUST_LOG=info ./target/release/otto &> "$OTTO_LOG" &
OTTO_PID=$!

for _ in $(seq 1 90); do
  grep -aq "D-Bus service started at org.otto.ScreenCast" "$OTTO_LOG" 2>/dev/null && break
  if ! kill -0 "$OTTO_PID" 2>/dev/null; then
    echo "!! otto died during startup:"
    grep -av "1P:\|op-app\|op-core\|Sentry\|Gdk-Message\|ozone" "$OTTO_LOG" | tail -15
    exit 1
  fi
  sleep 1
done

# socket + node come straight from otto's log (they change every run)
strip() { sed 's/\x1b\[[0-9;]*m//g'; }
WL=$(grep -a "Listening on wayland socket" "$OTTO_LOG" | strip | grep -oP 'name="\K[^"]+' | head -1)
WL="${WL:-wayland-1}"

if [ "$MODE" = "virtual" ]; then
  NODE=$(grep -a "Virtual output 'virtual-1' started" "$OTTO_LOG" | strip | grep -oP 'PipeWire node \K[0-9]+' | head -1)
  if [ -z "$NODE" ]; then
    echo "!! virtual-1 has no PipeWire node (is it enabled in otto_config.toml?)"
    cleanup
  fi
  ARGS=(--node "$NODE" --output virtual-1)
  echo "serving virtual-1 (PipeWire node $NODE)"
else
  ARGS=(--connector eDP-1)
  echo "serving physical eDP-1"
fi

# ── bridge ────────────────────────────────────────────────────────────────
# --tls unless OTTO_RDP_NOTLS=1: mstsc and the Windows App / iOS / Android
# clients REQUIRE it (they won't run graphics over plain-RDP security).
# OTTO_RDP_BITMAP=1 forces the legacy RemoteFX/RLE path (isolates H.264 issues).
[ -z "${OTTO_RDP_NOTLS:-}" ] && ARGS+=(--tls)
[ -n "${OTTO_RDP_BITMAP:-}" ] && ARGS+=(--bitmap)
echo "starting bridge -> $RDP_LOG   (args: ${ARGS[*]})"
# For an EGFX/DVC PDU trace on a failed connect, run with e.g.
#   RUST_LOG="info,ironrdp_egfx=trace,ironrdp_dvc=trace,ironrdp_server=debug" ./run-rdp.sh …
# NB: keep this block a single unbroken backslash-continuation — a comment
# between the lines silently drops the env vars below it.
WAYLAND_DISPLAY="$WL" \
DBUS_SESSION_BUS_ADDRESS="unix:path=/run/user/$(id -u)/bus" \
XDG_RUNTIME_DIR="/run/user/$(id -u)" \
RUST_LOG="${RUST_LOG:-info,otto_rdp=info}" \
OTTO_RDP_FPS="${OTTO_RDP_FPS:-30}" \
  ./target/release/otto-rdp "${ARGS[@]}" --listen "0.0.0.0:$PORT" &> "$RDP_LOG" &
RDP_PID=$!

sleep 4
if ! kill -0 "$RDP_PID" 2>/dev/null; then
  echo "!! bridge died:"
  tail -12 "$RDP_LOG" | strip
  cleanup
fi

IP=$(ip -4 -o addr show scope global 2>/dev/null | awk '{print $4}' | cut -d/ -f1 | head -1)
echo
echo "  connect:  xfreerdp3 /v:${IP:-<ip>}:$PORT /gfx:AVC420 /cert:ignore   (H.264)"
echo "            mobile/mstsc: just point it at ${IP:-<ip>}:$PORT (TLS on)"
echo "  otto=$OTTO_PID  bridge=$RDP_PID  (Ctrl+C to stop)"
echo "  avoid Super+Q / Ctrl+Alt+Backspace — they quit otto"
echo

# ── live status; report why otto goes away ────────────────────────────────
while kill -0 "$OTTO_PID" 2>/dev/null; do
  frames=$(grep -ac "captured frame" "$RDP_LOG" 2>/dev/null)
  subs=$(grep -a "RDP subscriber" "$RDP_LOG" 2>/dev/null | tail -1 | grep -oP '\d+(?= RDP subscriber)' || echo 0)
  rss=$(ps -o rss= -p "$RDP_PID" 2>/dev/null | tr -d ' ')
  printf "\r  frames=%-6s subscribers=%-3s bridge_rss=%sMB   " \
    "${frames:-0}" "${subs:-0}" "$((${rss:-0}/1024))"
  sleep 5
done

echo
echo "=== OTTO EXITED ==="
if grep -aq "keyboard shortcut activated" "$OTTO_LOG"; then
  echo ">> cause: QUIT SHORTCUT (Super+Q or Ctrl+Alt+Backspace)"
elif dmesg 2>/dev/null | grep -qi "killed process.*otto"; then
  echo ">> cause: OOM-KILLED"
else
  echo ">> cause: unknown — last otto lines:"
fi
grep -av "1P:\|op-app\|op-core\|Sentry\|Gdk-Message\|ozone\|Gtk:" "$OTTO_LOG" | strip | tail -8
kill "$RDP_PID" 2>/dev/null
