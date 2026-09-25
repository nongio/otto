#!/usr/bin/env bash
# Pin PKGBUILD-nightly-bin to one nightly tarball.
#
#   scripts/packaging/make-nightly-pkgbuild.sh <tarball> [output-dir]
#
# The nightly tarball is published under a URL that never changes, so the
# PKGBUILD in the repository cannot say which build it installs. This fills
# in the two things that do: pkgver, read from the VERSION file inside the
# tarball (the same value make-arch-tarball.sh wrote there), and the
# tarball's sha256. CI publishes the result beside the tarball, and
# `makepkg -p PKGBUILD-nightly-bin -si` on that copy installs exactly the
# build it was made for — a tarball from another night fails the checksum
# instead of installing under the wrong version.
set -euo pipefail

tarball="${1:?usage: make-nightly-pkgbuild.sh <tarball> [output-dir]}"
outdir="${2:-$(dirname "$tarball")}"
# Both relative to the caller, resolved before the cd below.
tarball=$(cd "$(dirname "$tarball")" && pwd)/$(basename "$tarball")
mkdir -p "$outdir"
outdir=$(cd "$outdir" && pwd)

cd "$(dirname "$0")/../.."

# The VERSION file at the tarball's top level, by exact path.
pkgver=$(tar -xzOf "$tarball" "$(tar -tzf "$tarball" | head -1)VERSION")
[ -n "$pkgver" ] || { echo "no VERSION file in $tarball" >&2; exit 1; }
# What the tarball says it is has to be a pkgver makepkg accepts: no
# hyphens, no colons, no whitespace.
case "$pkgver" in
    *[!A-Za-z0-9._+]*) echo "not a pkgver: $pkgver" >&2; exit 1 ;;
esac
sha256=$(sha256sum "$tarball" | cut -d' ' -f1)

# The template is read while the result is written: writing it over
# itself would truncate it first, and publish an empty file.
if [ "$outdir" = "$PWD" ]; then
    echo "output directory is the repository root, which holds the template" >&2
    exit 1
fi
sed -e "s/^pkgver=.*/pkgver=$pkgver/" \
    -e "s/^sha256sums=.*/sha256sums=(\"$sha256\")/" \
    PKGBUILD-nightly-bin > "$outdir/PKGBUILD-nightly-bin"

# Both substitutions have to have landed; a template that changed shape
# would otherwise publish a PKGBUILD that still says pkgver=0.
grep -q "^pkgver=$pkgver\$" "$outdir/PKGBUILD-nightly-bin"
grep -q "^sha256sums=(\"$sha256\")\$" "$outdir/PKGBUILD-nightly-bin"

echo "$outdir/PKGBUILD-nightly-bin"
