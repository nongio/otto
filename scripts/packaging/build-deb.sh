#!/usr/bin/env bash
# Build the Debian package from an already-built target/release.
#
#   scripts/packaging/build-deb.sh              # a nightly: 1.4.1+nightly.r123.gabc1234-1
#   OTTO_NIGHTLY=0 scripts/packaging/build-deb.sh   # a release: 1.4.1-1
#
# CI calls this, and so can a packaging change being tested locally. The
# version is the one part that is not in [package.metadata.deb]: a nightly
# has to carry its build in the version, or apt sees the release it was built
# from as "already the newest version" and installing it does nothing. See
# scripts/packaging/version.sh for the spelling.
set -euo pipefail

cd "$(dirname "$0")/../.."

args=(--no-build --no-strip)
if [ "${OTTO_NIGHTLY:-1}" = 1 ]; then
    # --deb-version replaces the whole string, revision included.
    args+=(--deb-version "$(scripts/packaging/version.sh deb)-1")
fi
cargo deb "${args[@]}"
