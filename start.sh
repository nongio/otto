#!/bin/bash
# Quick start for GPU profiling with perf counters
# Log goes to otto_perf.log — monitor with: tail -f otto_perf.log

set -e

cargo build --release --features "perf-counters"

echo "Starting Otto (udev) with perf-counters..."
echo "Monitor with:  tail -f otto_perf.log"
echo "Filter scene:  grep 'perf.scene' otto_perf.log"

RUST_LOG="debug" target/release/otto --tty-udev > otto_perf.log
