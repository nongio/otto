#!/usr/bin/env bash
# Stage the WhiteSur icon theme that every Otto package ships as its default.
#
#   scripts/packaging/fetch-whitesur.sh [output-dir]     # default: target/whitesur
#
# WhiteSur is not packaged by Debian or Fedora, and only in the AUR on Arch,
# so a first install would otherwise come up with Adwaita/hicolor fallbacks.
# The packages carry it instead: build-deb.sh, build-rpm.sh,
# make-arch-tarball.sh and PKGBUILD-git all call this script, so the pinned
# tag below is the one place to bump it.
#
# Leaves in the output directory:
#   WhiteSur/, WhiteSur-light/, WhiteSur-dark/
#                   the themes' regular files only
#   links.tar.gz    every symlink in the three themes, relative to /usr/share/icons
#   COPYING         WhiteSur's licence (GPL-3.0)
#
# The symlinks travel apart because neither package tool can carry them:
# cargo-generate-rpm copies the file a link points at, and cargo-deb copies
# a linked directory's contents (WhiteSur-light links whole directories into
# WhiteSur). Either way the theme triples in size. The deb and rpm install
# links.tar.gz with resources/whitesur-links on install and remove it before
# erase; the PKGBUILDs extract it into the package, so pacman owns the links.
#
# Running it again with the same pin does nothing.
set -euo pipefail

TAG=2026-09-10
SHA256=406c9cd59705583f1754b0eaca96cc48bafda042b88ef143f16d8ae1820ecd95
URL="https://codeload.github.com/vinceliuice/WhiteSur-icon-theme/tar.gz/refs/tags/$TAG"

cd "$(dirname "$0")/../.."
outdir="${1:-target/whitesur}"

if [ "$(cat "$outdir/.tag" 2>/dev/null)" = "$TAG" ]; then
    exit 0
fi

tmpdir=$(mktemp -d)
trap 'rm -rf "$tmpdir"' EXIT

curl -fsSL --retry 3 --retry-delay 5 -o "$tmpdir/whitesur.tar.gz" "$URL"
echo "$SHA256  $tmpdir/whitesur.tar.gz" | sha256sum -c --quiet -
tar -xzf "$tmpdir/whitesur.tar.gz" -C "$tmpdir"

rm -rf "$outdir"
mkdir -p "$outdir"
outdir=$(cd "$outdir" && pwd)
# The installer only copies, seds and links, then runs gtk-update-icon-cache
# under `set -e` after each theme. That is missing from a minimal build
# environment (the Arch container in CI), which stopped the installer after
# the first theme. The packages want no cache anyway — each distribution
# rebuilds icon caches when a package adds files under /usr/share/icons — so
# a no-op stands in for it everywhere.
mkdir "$tmpdir/bin"
printf '#!/bin/sh\nexit 0\n' > "$tmpdir/bin/gtk-update-icon-cache"
chmod 755 "$tmpdir/bin/gtk-update-icon-cache"
if ! (cd "$tmpdir/WhiteSur-icon-theme-$TAG" &&
      PATH="$tmpdir/bin:$PATH" bash install.sh -d "$outdir") > "$tmpdir/install.log" 2>&1; then
    cat "$tmpdir/install.log" >&2
    echo "fetch-whitesur: WhiteSur's install.sh failed" >&2
    exit 1
fi
for theme in WhiteSur WhiteSur-light WhiteSur-dark; do
    if [ ! -f "$outdir/$theme/index.theme" ]; then
        cat "$tmpdir/install.log" >&2
        echo "fetch-whitesur: $theme was not installed" >&2
        exit 1
    fi
    rm -f "$outdir/$theme/icon-theme.cache"
done
# Sorted and with fixed owner and time, so the same pin gives the same tar.
(
    cd "$outdir"
    find WhiteSur WhiteSur-light WhiteSur-dark -type l | LC_ALL=C sort > "$tmpdir/links"
    tar -cf - --no-recursion --owner=0 --group=0 --numeric-owner \
        --mtime=@0 -T "$tmpdir/links" | gzip -9n > links.tar.gz
    xargs -d '\n' rm -f < "$tmpdir/links"
)
install -m644 "$tmpdir/WhiteSur-icon-theme-$TAG/COPYING" "$outdir/COPYING"
echo "$TAG" > "$outdir/.tag"
