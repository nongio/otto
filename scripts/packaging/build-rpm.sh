#!/usr/bin/env bash
# Build the RPM package from an already-built target/release.
#
#   scripts/packaging/build-rpm.sh              # a nightly: 1.4.1^nightly.r123.gabc1234-1
#   OTTO_NIGHTLY=0 scripts/packaging/build-rpm.sh   # a release: 1.4.1-1
#
# CI calls this, and so can a packaging change being tested locally. The
# release version is the one in [package.metadata.generate-rpm]; a nightly
# overrides it so that dnf sees it as newer than the release it was built
# from and older than the next one. See scripts/packaging/version.sh.
set -euo pipefail

cd "$(dirname "$0")/../.."

args=()
if [ "${OTTO_NIGHTLY:-1}" = 1 ]; then
    args+=(--set-metadata "version = \"$(scripts/packaging/version.sh rpm)\"")
fi
cargo generate-rpm "${args[@]}"
