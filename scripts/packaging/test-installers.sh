#!/usr/bin/env bash
# Install every Otto package on a clean image of the distribution it targets,
# and check that it installs, links and runs.
#
#   scripts/packaging/test-installers.sh                 # all four, from the release
#   scripts/packaging/test-installers.sh deb rpm         # only these
#   OTTO_TAG=v1.0.0 scripts/packaging/test-installers.sh # a specific release
#   OTTO_LOCAL=1 scripts/packaging/test-installers.sh deb rpm
#
# Targets: deb, rpm, arch-bin, arch-nightly.
#
# By default the packages come from the GitHub release named by OTTO_TAG,
# because those are the artifacts CI built and users install. That matters for
# more than provenance: a locally built package on Arch carries binaries
# linked against Arch's glibc, which will not run on Ubuntu 24.04 or Fedora —
# so a local package can be checked for layout but never for "does it run".
#
# OTTO_LOCAL=1 uses target/debian and target/generate-rpm instead. Use it to
# iterate on packaging metadata; the run and linkage checks will fail for the
# glibc reason above, and that failure is not about your change.
set -euo pipefail

cd "$(dirname "$0")/../.."
repo=$PWD

OTTO_TAG="${OTTO_TAG:-$(git describe --tags --abbrev=0)}"
OTTO_LOCAL="${OTTO_LOCAL:-0}"
engine="${OTTO_CONTAINER_ENGINE:-$(command -v docker || command -v podman)}"
[[ -n "$engine" ]] || { echo "need docker or podman" >&2; exit 1; }

targets=("$@")
[[ ${#targets[@]} -gt 0 ]] || targets=(deb rpm arch-bin arch-nightly)

work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

# Build logs outlive the run; a failure is only useful if you can read it.
logdir="${OTTO_LOG_DIR:-$work/logs}"
mkdir -p "$logdir"

echo "engine: $engine"
echo "source: $([[ "$OTTO_LOCAL" == 1 ]] && echo "local build" || echo "release $OTTO_TAG")"
echo

# Stage a build context holding only what the Dockerfiles COPY, so the whole
# repo (and its multi-gigabyte target/) is not shipped to the daemon.
ctx="$work/ctx"
mkdir -p "$ctx/scripts/packaging" "$ctx/target/debian" "$ctx/target/generate-rpm"
cp "$repo/scripts/packaging/verify-install.sh" "$ctx/scripts/packaging/"
cp "$repo/PKGBUILD" "$repo/PKGBUILD-nightly-bin" "$ctx/"

fetch() {  # fetch <glob> <destination dir>
    gh release download "$OTTO_TAG" --repo nongio/otto \
        --pattern "$1" --dir "$2" --clobber
}

skip_run=0
if [[ "$OTTO_LOCAL" == 1 ]]; then
    cp "$repo"/target/debian/*.deb "$ctx/target/debian/" 2>/dev/null || true
    cp "$repo"/target/generate-rpm/*.rpm "$ctx/target/generate-rpm/" 2>/dev/null || true
    skip_run=1
else
    for t in "${targets[@]}"; do
        case "$t" in
            deb) fetch '*.deb' "$ctx/target/debian" ;;
            rpm) fetch '*.rpm' "$ctx/target/generate-rpm" ;;
        esac
    done
fi

# makepkg uses a source file already present in the build directory instead of
# downloading it, and sha256sums is SKIP, so dropping the tarball into the
# context is enough to point PKGBUILD at a locally built release.
stage_local_tarball() {
    echo "assembling the release tarball from the working tree..."
    "$repo/scripts/packaging/make-arch-tarball.sh" "$ctx" >/dev/null
    skip_run=1   # working-tree binaries, built against this machine's glibc
    echo "staged: $(cd "$ctx" && ls otto-*.tar.gz | tr '\n' ' ')"
}

declare -A results
run() {  # run <name> <dockerfile> [--build-arg ...]
    local name=$1 dockerfile=$2; shift 2
    echo "=============================================================="
    echo "== $name"
    echo "=============================================================="
    # --progress=plain, not --quiet: the whole point of a failure here is the
    # output of the step that failed, and buildkit's default renderer throws
    # it away. Logs are kept in OTTO_LOG_DIR for reading afterwards.
    if "$engine" build --progress=plain --no-cache=false \
            --build-arg "SKIP_RUN=$skip_run" \
            -f "$repo/scripts/packaging/$dockerfile" "$@" "$ctx" \
            > "$logdir/$name.log" 2>&1; then
        results[$name]=PASS
        echo "PASS"
    else
        results[$name]=FAIL
        echo "FAIL — see $logdir/$name.log"
        # Show the failing step's own output: everything after the last
        # "ERROR:" marker is buildkit's summary, what precedes it is the step.
        grep -nE '^#[0-9]+ ' "$logdir/$name.log" | tail -60 | sed 's/^/    /'
    fi
    echo
}

for t in "${targets[@]}"; do
    case "$t" in
        deb)          run deb          Dockerfile.deb ;;
        rpm)          run rpm          Dockerfile.rpm ;;
        arch-bin)     run arch-bin     Dockerfile.arch-bin ;;
        arch-nightly) run arch-nightly Dockerfile.arch-bin \
                          --build-arg PKGBUILD_FILE=PKGBUILD-nightly-bin ;;
        # Assemble the release tarball from the working tree with the same
        # script CI uses, and build the binary PKGBUILD against it. This is
        # the only target that checks the tarball's contents against what the
        # PKGBUILDs install out of it, so it catches a file added to one and
        # not the other before a release does.
        arch-local)   stage_local_tarball; run arch-local Dockerfile.arch-bin ;;
        *) echo "unknown target: $t" >&2; exit 2 ;;
    esac
done

echo "=============================================================="
status=0
for name in "${!results[@]}"; do
    printf '%-16s %s\n' "$name" "${results[$name]}"
    [[ "${results[$name]}" == PASS ]] || status=1
done
exit $status
