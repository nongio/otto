#!/usr/bin/env bash
# The version every package built from this checkout carries, in each
# package manager's spelling.
#
#   scripts/packaging/version.sh            # KEY=value lines, one per spelling
#   scripts/packaging/version.sh deb        # one spelling on its own
#
# A release package carries the workspace version as it is. A nightly has
# to say which build it is, and — the part that matters — sort *above* the
# release it was built from and *below* the next one, or a package manager
# treats every nightly as "already installed" and an update does nothing.
# The three formats spell "release plus commits" differently, so this script
# is the one place that knows all three:
#
#   workspace   1.4.1              (Cargo.toml [workspace.package])
#   snapshot    r123.gabc1234      (commits on this branch, and the commit)
#   deb         1.4.1+nightly.r123.gabc1234    '+' sorts after the release
#   rpm         1.4.1^nightly.r123.gabc1234    '^' is rpm's post-release marker
#   arch        1.4.1.r123.gabc1234            an extra component sorts after
#
# The commit count is what makes one nightly newer than the last: pacman,
# dpkg and rpm all compare it as a number, and it only ever grows on main.
# The short hash is there for people, not the comparison.
#
# A prerelease (1.0.0-rc.2) needs its hyphen respelled: dpkg and rpm read
# '~' as "sorts before", which is what a prerelease wants; pkgver may not
# contain a hyphen at all, so Arch just drops it.
#
# OTTO_NIGHTLY=0 asks for the release spellings: the plain workspace version
# for deb and rpm, and the PKGBUILD's pkgver for Arch. CI sets it from the
# ref it is building; the default is a nightly, which is what any checkout
# that is not a release tag is.
set -euo pipefail

cd "$(dirname "$0")/../.."

# Read the version out of [workspace.package]. Not `head -1` on the first
# `version = ` line: the crate inherits with `version.workspace = true`, so
# the first such line in the file belongs to a [dependencies.*] table and
# names that dependency.
workspace=$(sed -n '/^\[workspace.package\]/,/^\[/p' Cargo.toml | sed -n 's/^version = "\(.*\)"/\1/p' | head -1)
if [ -z "$workspace" ]; then
    echo "no [workspace.package] version found in Cargo.toml" >&2
    exit 1
fi

nightly="${OTTO_NIGHTLY:-1}"

if [ "$nightly" = 1 ]; then
    # `git rev-list --count` needs the whole history: a shallow clone counts
    # the commits it has, which is one, and every nightly comes out as r1.
    if [ "$(git rev-parse --is-shallow-repository)" = true ]; then
        echo "shallow clone: the commit count would be wrong (fetch-depth: 0)" >&2
        exit 1
    fi
    snapshot="r$(git rev-list --count HEAD).g$(git rev-parse --short=7 HEAD)"
    deb="${workspace//-/\~}+nightly.$snapshot"
    rpm="${workspace//-/\~}^nightly.$snapshot"
    arch="${workspace//-/}.$snapshot"
else
    snapshot=""
    deb="${workspace//-/\~}"
    rpm="${workspace//-/\~}"
    arch="${workspace//-/}"
fi

case "${1:-}" in
    "")
        echo "OTTO_VERSION=$workspace"
        echo "OTTO_SNAPSHOT=$snapshot"
        echo "OTTO_DEB_VERSION=$deb"
        echo "OTTO_RPM_VERSION=$rpm"
        echo "OTTO_PKGVER=$arch"
        ;;
    workspace) echo "$workspace" ;;
    snapshot)  echo "$snapshot" ;;
    deb)       echo "$deb" ;;
    rpm)       echo "$rpm" ;;
    arch)      echo "$arch" ;;
    *) echo "usage: $0 [workspace|snapshot|deb|rpm|arch]" >&2; exit 2 ;;
esac
