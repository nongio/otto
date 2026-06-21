#!/usr/bin/env bash
# Otto GPU/CPU perf sampler — one scenario window.
# Usage: perf_sample.sh <label> <seconds>
# Captures, over <seconds>:
#   - intel_gpu_top aggregate engine % (if PMU accessible)
#   - per-process DRM fdinfo engine busy % (render/video/blitter) for otto + mpv + components
#   - GPU act freq + RC6 residency delta (aggregate idle proxy)
#   - top CPU% for otto + mpv + components
# Output: compact, parseable summary to stdout.

set -u
LABEL="${1:?label}"
SECS="${2:-8}"
CARD=card1
DRM_DEV=/dev/dri/card1
PROCS_RE='otto|mpv|otto-bar|otto-islands|apps-manager|topbar'

echo "########## SCENARIO: $LABEL (${SECS}s) ##########"

# --- helper: read summed engine ns from a pid's drm fdinfo ---
# echoes: "render_ns video_ns blitter_ns copy_ns" (0 if absent)
fdinfo_engines() {
  local pid=$1
  awk '
    /drm-engine-render:/   {r+=$2}
    /drm-engine-video:/    {v+=$2}
    /drm-engine-video-enhance:/ {v+=$2}
    /drm-engine-blitter:/  {b+=$2}
    /drm-engine-copy:/     {c+=$2}
    END {printf "%d %d %d %d", r+0, v+0, b+0, c+0}
  ' /proc/$pid/fdinfo/* 2>/dev/null
}

mapfile -t PIDS < <(pgrep -f "$PROCS_RE" 2>/dev/null | sort -u)
declare -A NAME T0
for pid in "${PIDS[@]}"; do
  [ -d /proc/$pid ] || continue
  NAME[$pid]=$(tr -d '\0' </proc/$pid/comm 2>/dev/null)
  T0[$pid]=$(fdinfo_engines "$pid")
done

# --- aggregate: rc6 + freq snapshot start ---
RC6_0=$(cat /sys/class/drm/$CARD/power/rc6_residency_ms 2>/dev/null || echo 0)
FREQ_SAMPLES=/tmp/perf_freq_$$; : >"$FREQ_SAMPLES"

# --- intel_gpu_top in background (CSV) if PMU allowed ---
# Preflight: check for actual numeric output (timeout's exit code is unreliable).
IGT_OUT=/tmp/perf_igt_$$; IGT_OK=0
if timeout 2 intel_gpu_top -d "drm:$DRM_DEV" -c -s 500 2>/dev/null | grep -qE '^[0-9]'; then
  IGT_OK=1
  intel_gpu_top -d "drm:$DRM_DEV" -c -s 500 >"$IGT_OUT" 2>/dev/null &
  IGT_PID=$!
fi

# --- top in background for CPU ---
TOP_OUT=/tmp/perf_top_$$
top -b -d 1 -n "$SECS" >"$TOP_OUT" 2>/dev/null &
TOP_PID=$!

# --- sample freq during window ---
for ((i=0; i<SECS*2; i++)); do
  cat /sys/class/drm/$CARD/gt_act_freq_mhz 2>/dev/null >>"$FREQ_SAMPLES"
  sleep 0.5
done

wait "$TOP_PID" 2>/dev/null
[ "$IGT_OK" = 1 ] && { kill "$IGT_PID" 2>/dev/null; wait "$IGT_PID" 2>/dev/null; }

# --- aggregate end ---
RC6_1=$(cat /sys/class/drm/$CARD/power/rc6_residency_ms 2>/dev/null || echo 0)

# ===== REPORT: per-process GPU engine utilization (fdinfo delta) =====
echo "--- GPU per-process (fdinfo, % of wall over ${SECS}s) ---"
printf "%-16s %8s %8s %8s\n" "process" "render%" "video%" "blit%"
WALL_NS=$((SECS * 1000000000))
for pid in "${PIDS[@]}"; do
  [ -d /proc/$pid ] || continue
  read r1 v1 b1 c1 <<<"$(fdinfo_engines "$pid")"
  read r0 v0 b0 c0 <<<"${T0[$pid]:-0 0 0 0}"
  dr=$(( r1 - r0 )); dv=$(( v1 - v0 )); db=$(( (b1 - b0) + (c1 - c0) ))
  pr=$(awk "BEGIN{printf \"%.1f\", $dr*100/$WALL_NS}")
  pv=$(awk "BEGIN{printf \"%.1f\", $dv*100/$WALL_NS}")
  pb=$(awk "BEGIN{printf \"%.1f\", $db*100/$WALL_NS}")
  # only print rows with any activity OR known compositor procs
  printf "%-16s %8s %8s %8s\n" "${NAME[$pid]}($pid)" "$pr" "$pv" "$pb"
done

# ===== REPORT: aggregate GPU =====
echo "--- GPU aggregate ---"
if [ -s "$FREQ_SAMPLES" ]; then
  awk '{s+=$1; n++; if($1>mx)mx=$1} END{printf "act_freq MHz: avg %.0f / max %.0f (RPn..RP0 idle..max)\n", s/n, mx}' "$FREQ_SAMPLES"
fi
RC6_DELTA=$(( RC6_1 - RC6_0 ))
RC6_PCT=$(awk "BEGIN{p=$RC6_DELTA*100/($SECS*1000); if(p>100)p=100; printf \"%.1f\", p}")
echo "RC6 (GPU sleep) residency: ${RC6_PCT}% of window  (high% = GPU mostly idle)"
if [ "$IGT_OK" = 1 ] && [ -s "$IGT_OUT" ]; then
  echo "--- intel_gpu_top aggregate (avg, skipping first sample) ---"
  # Columns of interest by exact header name.
  awk -F',' '
    NR==1{ for(i=1;i<=NF;i++){ g=$i; gsub(/^ +| +$/,"",g); h[i]=g } }
    NR>2{ for(i=1;i<=NF;i++){ s[i]+=$i; c[i]++ } }
    END{
      want="Freq MHz act|RC6 %|Power W gpu|RCS %|BCS %|VCS %|VECS %"
      for(i=1;i<=length(h);i++) if(h[i] ~ ("^("want")$") && c[i]>0)
        printf "  %-14s %.1f\n", h[i]":", s[i]/c[i]
    }' "$IGT_OUT"
else
  echo "intel_gpu_top: PMU not accessible (run: sudo sh -c 'echo -1 > /proc/sys/kernel/perf_event_paranoid')"
fi

# ===== REPORT: CPU =====
echo "--- CPU% (top, avg over window) ---"
printf "%-16s %8s %8s\n" "process" "cpu%avg" "cpu%max"
for pat in otto mpv otto-bar otto-islands apps-manager topbar; do
  awk -v p="$pat" '
    $NF==p || $(NF) ~ ("^" p "$") {
      cpu=$9+0; s+=cpu; n++; if(cpu>mx)mx=cpu
    }
    END{ if(n>0) printf "%-16s %8.1f %8.1f\n", p, s/n, mx }' "$TOP_OUT"
done

rm -f "$FREQ_SAMPLES" "$IGT_OUT" "$TOP_OUT"
echo "########## END $LABEL ##########"
