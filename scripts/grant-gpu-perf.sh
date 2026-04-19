#!/usr/bin/env bash
set -euo pipefail

BIN="$(command -v intel_gpu_top)"
if [[ -z "$BIN" ]]; then
    echo "intel_gpu_top not found. Install intel-gpu-tools first."
    exit 1
fi

echo "Granting CAP_PERFMON to: $BIN"
pkexec setcap cap_perfmon=ep "$BIN"

echo
echo "Verifying capability:"
getcap "$BIN"

echo
echo "Done. Test with: intel_gpu_top -s 1000"
