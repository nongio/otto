#!/bin/sh
# Stress a running Otto session for deadlocks, from a terminal inside it.
#
# Opens and closes scrolling foot windows, minimises and restores them (the
# genie runs on a tokio task), maximises them, opens exposé and the app
# switcher and switches workspaces, as fast as the session takes it. After
# every action it checks that Otto still answers a Wayland request; if Otto
# stops answering, and its main thread sits blocked on a lock, it saves every
# thread's stack and stops. Otto itself is left as it is.
#
#   scripts/stress-live.sh [seconds]        # default 300
#
# Run `sudo -v` first: attaching gdb to Otto needs root, and a frozen session
# cannot ask for a password. Without it the script still detects the hang and
# says what to run from another TTY.
#
# Shortcuts are the defaults (`Prior` exposé, `Ctrl+Tab` switcher,
# `Ctrl+1/2` workspaces); set STRESS_EXPOSE / STRESS_SWITCH /
# STRESS_WS_MODS if your config binds others. Keys go to whatever is focused,
# so leave nothing open that a stray PageUp or Ctrl+Tab would upset.

DURATION=${1:-300}
APP=otto-stress-live
EXPOSE=${STRESS_EXPOSE:-Prior}
SWITCH=${STRESS_SWITCH:-Tab}
WS_MODS=${STRESS_WS_MODS:-CTRL}
OUT=${STRESS_OUT:-${XDG_RUNTIME_DIR:-/tmp}/otto-stress-live}
MAX_WINDOWS=6

[ -n "$WAYLAND_DISPLAY" ] || { echo "run this inside the Otto session"; exit 1; }
for tool in wlrctl foot gdb; do
  command -v $tool >/dev/null || { echo "needs $tool"; exit 1; }
done
OTTO=$(pgrep -u "$(id -u)" -x otto | head -1)
[ -n "$OTTO" ] || { echo "no otto process"; exit 1; }
mkdir -p "$OUT"
echo "stressing otto pid $OTTO for ${DURATION}s; output in $OUT"

PIDS=""
cleanup() {
  for p in $PIDS; do kill "$p" 2>/dev/null; done
}
trap cleanup EXIT INT TERM

count() { set -- $PIDS; echo $#; }
rand() { echo $(( $(od -An -N2 -tu2 /dev/urandom) % $1 )); }
key() { timeout 3 wlrctl keyboard type "$@" >/dev/null 2>&1; }
top() { timeout 3 wlrctl toplevel "$@" app_id:$APP >/dev/null 2>&1; }

open_window() {
  foot --app-id=$APP --title="stress $(date +%s%N)" \
    sh -c 'while :; do seq 1 4000; ls -l /usr/bin | head -200; done' \
    >/dev/null 2>&1 &
  PIDS="$PIDS $!"
}

close_window() {
  set -- $PIDS
  [ $# -gt 0 ] || return
  kill "$1" 2>/dev/null
  shift
  PIDS="$*"
}

# Otto answers a sync if its loop turns; a hang is a loop that does not, with
# the main thread asleep on a futex rather than in epoll.
alive() {
  timeout 5 wlrctl toplevel find app_id:$APP >/dev/null 2>&1
  [ $? -ne 124 ]
}

hang() {
  wchan=$(cat /proc/$OTTO/task/$OTTO/wchan 2>/dev/null)
  stamp=$(date +%m%d-%H%M%S)
  file="$OUT/otto-hang-$stamp.txt"
  echo "OTTO STOPPED ANSWERING (main thread wchan: $wchan) after $STEPS actions, last: $LAST"
  if sudo -n true 2>/dev/null; then
    sudo -n timeout 120 gdb -p "$OTTO" -batch -nx -iex "set debuginfod enabled off" \
      -ex "set pagination off" -ex "thread apply all bt 60" > "$file" 2>&1
    echo "every thread's stack: $file"
  else
    echo "no cached sudo; from another TTY run:"
    echo "  sudo gdb -p $OTTO -batch -ex 'thread apply all bt 60' > $file"
  fi
  trap - EXIT
  exit 2
}

STEPS=0
END=$(( $(date +%s) + DURATION ))
open_window
sleep 1
while [ "$(date +%s)" -lt "$END" ]; do
  kill -0 "$OTTO" 2>/dev/null || { echo "otto exited (crash?) after $STEPS actions, last: $LAST"; exit 3; }
  n=$(count)
  case $(rand 12) in
    0|1) if [ "$n" -lt $MAX_WINDOWS ]; then LAST=open; open_window; else LAST=close; close_window; fi ;;
    2)   LAST=close; [ "$n" -gt 1 ] && close_window ;;
    3)   LAST=minimize; top minimize ;;
    4)   LAST=restore; top focus ;;   # focusing a minimised window restores it
    5)   LAST=maximize; top maximize ;;
    6)   LAST=fullscreen; top fullscreen ;;
    7)   LAST=expose; key "$EXPOSE"; sleep 0.$(rand 6); key "$EXPOSE" ;;
    8)   LAST=switcher; key "$SWITCH" modifiers CTRL ;;
    9)   LAST=workspace; key 2 modifiers "$WS_MODS"; sleep 0.$(rand 5); key 1 modifiers "$WS_MODS" ;;
    *)   LAST=focus; top focus ;;
  esac
  STEPS=$((STEPS + 1))
  sleep 0.$(rand 3)
  alive || { sleep 5; alive || hang; }
  [ $((STEPS % 50)) -eq 0 ] && echo "$STEPS actions, $(count) windows, otto answering"
done
echo "no hang in ${DURATION}s ($STEPS actions)"
