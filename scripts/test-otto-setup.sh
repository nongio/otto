#!/usr/bin/env bash
# Run otto-setup on a clean Debian-family image and check what it says: the
# first-run case the tool exists for, which a developer's own machine can no
# longer show. The scenarios are in components/otto-setup/tests/clean-machine.
#
#   scripts/test-otto-setup.sh                            # ubuntu:24.04
#   OTTO_SETUP_BASE=debian:trixie scripts/test-otto-setup.sh
#   OTTO_NO_BUILD=1 scripts/test-otto-setup.sh            # binaries as they are
#
# It builds otto-setup and otto-agents (debug), copies them into the image
# and runs the scenarios there. The binaries are this machine's, so the image
# needs a glibc at least as new as the symbols they use; when the image is
# too old the first step says so and OTTO_SETUP_BASE picks a newer one.
# OTTO_CONTAINER_ENGINE picks docker or podman; OTTO_LOG_DIR keeps the log.
set -euo pipefail

cd "$(dirname "$0")/.."
repo=$PWD
tests="$repo/components/otto-setup/tests/clean-machine"

engine="${OTTO_CONTAINER_ENGINE:-$(command -v docker || command -v podman || true)}"
[[ -n "$engine" ]] || { echo "need docker or podman" >&2; exit 1; }
base="${OTTO_SETUP_BASE:-ubuntu:24.04}"
target="${CARGO_TARGET_DIR:-$repo/target}"

if [[ "${OTTO_NO_BUILD:-0}" != 1 ]]; then
    echo "building otto-setup and otto-agents..."
    cargo build -q -p otto-setup -p otto-agents
fi
for b in otto-setup otto-agents; do
    [[ -x "$target/debug/$b" ]] || { echo "no $target/debug/$b" >&2; exit 1; }
done

work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
logdir="${OTTO_LOG_DIR:-$work}"
mkdir -p "$logdir" "$work/bin"
log="$logdir/otto-setup-$(tr ':/' '--' <<<"$base").log"

cp "$target/debug/otto-setup" "$target/debug/otto-agents" "$work/bin/"
cp -r "$repo/resources/plugins/otto" "$work/plugins"
cp "$tests/check.sh" "$tests/screen.py" "$work/"

echo "engine: $engine"
echo "image:  $base"
echo
# --progress=plain keeps the steps' own output, which is the test's result.
# Only what the steps print is shown; the whole log is kept.
if "$engine" build --progress=plain --build-arg "BASE=$base" \
        -f "$tests/Dockerfile" "$work" 2>&1 | tee "$log" \
        | sed -nE 's/^#[0-9]+ [0-9.]+ //p'; then
    echo
    echo "PASS: otto-setup on $base"
else
    echo
    echo "FAIL: otto-setup on $base — see $log"
    grep -E 'ERROR' "$log" | tail -5 | sed 's/^/    /'
    exit 1
fi
