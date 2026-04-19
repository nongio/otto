#!/usr/bin/env bash
# Run a 10s perf + GPU snapshot of Otto, save to a directory.
# Usage: ./measure.sh <output-dir> [<scenario-note>]
set -uo pipefail
# pipefail off for `head` SIGPIPE tolerance when truncating perf report

OUTDIR="${1:?need output dir}"
NOTE="${2:-}"
mkdir -p "$OUTDIR"
cd "$OUTDIR"

OTTO_PID=$(cat /tmp/otto.pid 2>/dev/null) || { echo "no /tmp/otto.pid"; exit 1; }
[[ -d /proc/$OTTO_PID ]] || { echo "otto pid $OTTO_PID not running"; exit 1; }

echo "Scenario: $NOTE"
echo "Otto PID: $OTTO_PID"

# Snapshot of all wayland clients before the run
ps -ef --no-headers | awk -v wpid=$OTTO_PID '$3 == wpid {print}' > clients.txt
echo "$(wc -l < clients.txt) child processes of otto"

# top before
top -b -n 1 -p $OTTO_PID > top-before.txt
grep -E "^\s*$OTTO_PID" top-before.txt | awk '{printf "before: cpu=%s%% mem=%s%%\n", $9, $10}'

# Parallel: GPU + perf for 10s
timeout 10 intel_gpu_top -J -s 1000 > intel-gpu-top.json 2>&1 &
GPU_TASK=$!
perf record -F 99 -g -p $OTTO_PID -o perf.data sleep 10 2>perf-record.err | true

wait $GPU_TASK 2>/dev/null || true

# top after
top -b -n 1 -p $OTTO_PID > top-after.txt
grep -E "^\s*$OTTO_PID" top-after.txt | awk '{printf "after:  cpu=%s%% mem=%s%%\n", $9, $10}'

# Perf summary — write full report then truncate to avoid SIGPIPE
perf report -i perf.data --stdio --no-children --sort symbol 2>/dev/null \
    | grep -E '^\s+[0-9]+\.[0-9]+%' > perf-all.txt || true
head -20 perf-all.txt > perf-top20.txt
echo "--- top 5 hot symbols ---"
head -5 perf-top20.txt

# GPU summary
python3 << 'PY' > gpu-summary.txt
import json
buf = open('intel-gpu-top.json').read()
objs = []
depth = 0; start = None
for i, c in enumerate(buf):
    if c == '{':
        if depth == 0: start = i
        depth += 1
    elif c == '}':
        depth -= 1
        if depth == 0 and start is not None:
            try: objs.append(json.loads(buf[start:i+1]))
            except: pass
            start = None
print(f'samples: {len(objs)}')
if not objs: exit()

rcs, gpu_pwr, pkg_pwr, freq, rc6 = [], [], [], [], []
for o in objs[2:]:
    rcs.append(float(o.get('engines', {}).get('Render/3D', {}).get('busy', 0)))
    p = o.get('power', {})
    gpu_pwr.append(float(p.get('GPU', 0)))
    pkg_pwr.append(float(p.get('Package', 0)))
    freq.append(float(o.get('frequency', {}).get('actual', 0)))
    rc6.append(float(o.get('rc6', {}).get('value', 0)))

def stat(label, vals, fmt='6.2f'):
    if not vals: return
    print(f'{label:<13} avg={sum(vals)/len(vals):{fmt}}  min={min(vals):{fmt}}  max={max(vals):{fmt}}')

stat('RCS busy %',  rcs)
stat('GPU power W', gpu_pwr)
stat('Pkg power W', pkg_pwr)
stat('Freq MHz',    freq, '6.0f')
stat('RC6 %',       rc6)

print()
print('Per-client RCS avg:')
agg = {}
for o in objs[2:]:
    for cid, info in o.get('clients', {}).items():
        name = info.get('name', '?'); pid = info.get('pid', '?')
        rcs_v = float(info.get('engine-classes', {}).get('Render/3D', {}).get('busy', '0'))
        agg.setdefault((name, pid), []).append(rcs_v)
for (name, pid), vals in sorted(agg.items(), key=lambda x: -sum(x[1])/len(x[1])):
    avg = sum(vals)/len(vals)
    if avg > 0.05:
        print(f'  {name:<25} pid={pid:>7}  avg={avg:5.1f}%  max={max(vals):5.1f}%')
PY
echo "--- gpu summary ---"
cat gpu-summary.txt

# Otto alive after?
[[ -d /proc/$OTTO_PID ]] && echo "otto still alive" || echo "OTTO DIED DURING MEASUREMENT"

echo "saved: $OUTDIR"
