#!/usr/bin/env bash
# Install every Otto package on a clean image of the distribution it targets,
# and check that it installs, links, runs — and, for the nightlies, that it
# upgrades the release it was built from.
#
#   scripts/packaging/test-installers.sh                 # everything, from the releases
#   scripts/packaging/test-installers.sh deb rpm         # only these
#   OTTO_TAG=v1.4.1 scripts/packaging/test-installers.sh # a specific release
#   OTTO_LOCAL=1 scripts/packaging/test-installers.sh deb rpm
#
# Targets:
#   deb, rpm, arch-bin       the release named by OTTO_TAG, on a clean image
#   deb-nightly, rpm-nightly, arch-nightly
#                            that release, then the nightly on top of it: the
#                            nightly has to install as an upgrade
#   arch-local               the working tree's tarball and both PKGBUILDs
#                            that install out of it (see stage_local_tarball)
#
# By default the packages come from the GitHub releases, because those are
# the artifacts CI built and users install. That matters for more than
# provenance: a locally built package on Arch carries binaries linked against
# Arch's glibc, which will not run on Ubuntu 24.04 or Fedora — so a local
# package can be checked for layout but never for "does it run".
#
# OTTO_LOCAL=1 uses target/debian and target/generate-rpm instead. Use it to
# iterate on packaging metadata; the run and linkage checks will fail for the
# glibc reason above, and that failure is not about your change.
#
# OTTO_DEB_BASE, OTTO_RPM_BASE and OTTO_ARCH_BASE pick the images
# (ubuntu:22.04, say); OTTO_DOCKER_BUILD_ARGS is passed to every build
# (--network host behind a proxy, for instance).
set -euo pipefail

cd "$(dirname "$0")/../.."
repo=$PWD

OTTO_TAG="${OTTO_TAG:-$(git describe --tags --abbrev=0 --match 'v*')}"
OTTO_LOCAL="${OTTO_LOCAL:-0}"
engine="${OTTO_CONTAINER_ENGINE:-$(command -v docker || command -v podman)}"
[[ -n "$engine" ]] || { echo "need docker or podman" >&2; exit 1; }
# shellcheck disable=SC2206  # word-split on purpose: it is a list of flags
build_args=(${OTTO_DOCKER_BUILD_ARGS:-})

targets=("$@")
[[ ${#targets[@]} -gt 0 ]] || targets=(deb rpm arch-bin deb-nightly rpm-nightly arch-nightly)

work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

# Build logs outlive the run; a failure is only useful if you can read it.
logdir="${OTTO_LOG_DIR:-$work/logs}"
mkdir -p "$logdir"

echo "engine: $engine"
echo "source: $([[ "$OTTO_LOCAL" == 1 ]] && echo "local build" || echo "release $OTTO_TAG")"
echo

# Stage a build context holding only what the Dockerfiles COPY, so the whole
# repo (and its multi-gigabyte target/) is not shipped to the daemon. The
# *-before directories hold the package a target upgrades from; empty, the
# Dockerfiles skip that step.
ctx="$work/ctx"
stage_context() {
    rm -rf "$ctx"
    mkdir -p "$ctx/scripts/packaging" "$ctx/before" \
        "$ctx/target/debian" "$ctx/target/debian-before" \
        "$ctx/target/generate-rpm" "$ctx/target/generate-rpm-before"
    cp "$repo/scripts/packaging/verify-install.sh" "$ctx/scripts/packaging/"
    cp "$repo/PKGBUILD" "$ctx/"
}

# Every release publishes its packages under names that never change, so
# they can be fetched by name rather than through the GitHub API:
# otto-amd64.deb and otto-x86_64.rpm under a release tag, and the
# otto-nightly-* files plus the pinned PKGBUILD-nightly-bin under `nightly`.
fetch() {  # fetch <tag> <asset> <destination>
    curl -fsSL "https://github.com/nongio/otto/releases/download/$1/$2" -o "$3"
}

# The release packages, into the directory the Dockerfiles install from (or
# the one they upgrade from, when a nightly is under test).
stage_release() {  # stage_release <deb|rpm> <dir>
    if [[ "$OTTO_LOCAL" == 1 ]]; then
        case "$1" in
            deb) cp "$repo"/target/debian/*.deb "$2/" ;;
            rpm) cp "$repo"/target/generate-rpm/*.rpm "$2/" ;;
        esac
    else
        case "$1" in
            deb) fetch "$OTTO_TAG" otto-amd64.deb "$2/otto-amd64.deb" ;;
            rpm) fetch "$OTTO_TAG" otto-x86_64.rpm "$2/otto-x86_64.rpm" ;;
        esac
    fi
}

# makepkg uses a source file already present in the build directory instead of
# downloading it, so dropping the tarball into the context is enough to point
# a PKGBUILD at a locally built release. The release PKGBUILD looks for the
# versioned name; the nightly one, pinned to this very tarball by
# make-arch-tarball.sh, for a name carrying its pkgver.
stage_local_tarball() {
    echo "assembling the release tarball from the working tree..."
    "$repo/scripts/packaging/make-arch-tarball.sh" "$ctx" >/dev/null
    local pkgver
    pkgver=$(sed -n 's/^pkgver=//p' "$ctx/PKGBUILD-nightly-bin")
    cp "$ctx/otto-nightly-x86_64.tar.gz" "$ctx/otto-nightly-$pkgver-x86_64.tar.gz"
    echo "staged: $(cd "$ctx" && ls otto-*.tar.gz | tr '\n' ' ')"
}

declare -A results
run() {  # run <name> <dockerfile> <skip-run> [--build-arg ...]
    local name=$1 dockerfile=$2 skip_run=$3; shift 3
    echo "=============================================================="
    echo "== $name"
    echo "=============================================================="
    # --progress=plain, not --quiet: the whole point of a failure here is the
    # output of the step that failed, and buildkit's default renderer throws
    # it away. Logs are kept in OTTO_LOG_DIR for reading afterwards.
    if "$engine" build --progress=plain --no-cache=false \
            --build-arg "SKIP_RUN=$skip_run" "${build_args[@]}" \
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

base_arg() {  # base_arg <OTTO_*_BASE value>: a --build-arg, or nothing
    [[ -n "$1" ]] && printf -- '--build-arg\nBASE=%s\n' "$1" || true
}
mapfile -t deb_base < <(base_arg "${OTTO_DEB_BASE:-}")
mapfile -t rpm_base < <(base_arg "${OTTO_RPM_BASE:-}")
mapfile -t arch_base < <(base_arg "${OTTO_ARCH_BASE:-}")

# A locally built package cannot be run on the target image (its glibc is
# this machine's), so the run checks are skipped for it.
local_skip=$([[ "$OTTO_LOCAL" == 1 ]] && echo 1 || echo 0)

for t in "${targets[@]}"; do
    stage_context
    case "$t" in
        deb)
            stage_release deb "$ctx/target/debian"
            run deb Dockerfile.deb "$local_skip" "${deb_base[@]}" ;;
        rpm)
            stage_release rpm "$ctx/target/generate-rpm"
            run rpm Dockerfile.rpm "$local_skip" "${rpm_base[@]}" ;;
        arch-bin)
            run arch-bin Dockerfile.arch-bin 0 "${arch_base[@]}" ;;
        deb-nightly)
            stage_release deb "$ctx/target/debian-before"
            fetch nightly otto-nightly-amd64.deb "$ctx/target/debian/otto-nightly-amd64.deb"
            run deb-nightly Dockerfile.deb 0 "${deb_base[@]}" ;;
        rpm-nightly)
            stage_release rpm "$ctx/target/generate-rpm-before"
            fetch nightly otto-nightly-x86_64.rpm "$ctx/target/generate-rpm/otto-nightly-x86_64.rpm"
            run rpm-nightly Dockerfile.rpm 0 "${rpm_base[@]}" ;;
        arch-nightly)
            # The pinned PKGBUILD published beside the tarball, exactly what a
            # user downloads; the release PKGBUILD is what it upgrades.
            fetch nightly PKGBUILD-nightly-bin "$ctx/PKGBUILD-nightly-bin"
            cp "$repo/PKGBUILD" "$ctx/before/PKGBUILD"
            run arch-nightly Dockerfile.arch-bin 0 "${arch_base[@]}" \
                --build-arg PKGBUILD_FILE=PKGBUILD-nightly-bin ;;
        # Assemble the release tarball from the working tree with the same
        # script CI uses, and build both binary PKGBUILDs against it. This is
        # the only target that checks the tarball's contents against what the
        # PKGBUILDs install out of it, so it catches a file added to one and
        # not the other before a release does.
        arch-local)
            stage_local_tarball
            run arch-local Dockerfile.arch-bin 1 "${arch_base[@]}"
            run arch-local-nightly Dockerfile.arch-bin 1 "${arch_base[@]}" \
                --build-arg PKGBUILD_FILE=PKGBUILD-nightly-bin ;;
        *) echo "unknown target: $t" >&2; exit 2 ;;
    esac
done

echo "=============================================================="
status=0
for name in "${!results[@]}"; do
    printf '%-20s %s\n' "$name" "${results[$name]}"
    [[ "${results[$name]}" == PASS ]] || status=1
done
exit $status
