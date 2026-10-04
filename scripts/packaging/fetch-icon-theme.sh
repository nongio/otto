#!/usr/bin/env bash
# Stage the icon theme every Otto package ships as its default: MacTahoe,
# installed under Otto's own name.
#
#   scripts/packaging/fetch-icon-theme.sh [output-dir]     # default: target/icon-theme
#
# MacTahoe is not packaged by Debian or Fedora, and only in the AUR on Arch,
# so a first install would otherwise come up with Adwaita/hicolor fallbacks.
# The packages carry it instead: build-deb.sh, build-rpm.sh,
# make-arch-tarball.sh and PKGBUILD-git all call this script, so the pinned
# tag below is the one place to bump it.
#
# It is installed as Otto-MacTahoe rather than MacTahoe so that it never
# collides with a MacTahoe the user installed themselves — the AUR package,
# or upstream's install.sh run as root — whose files Otto's package would
# otherwise overwrite, conflict with, or delete on removal.
#
# Leaves in the output directory:
#   Otto-MacTahoe/, Otto-MacTahoe-light/, Otto-MacTahoe-dark/
#                       the themes' regular files only; each carries an
#                       XCursor set too, so the name works as cursor_theme
#   links-<tag>.tar.gz  every symlink in the three themes, relative to
#                       /usr/share/icons; named by tag so that an upgrade
#                       can still find the links an older package made
#   COPYING             MacTahoe's licence (GPL-3.0)
#   SOURCE              where the corresponding source is (GPL-3.0 §6: the
#                       cursors are compiled XCursor files)
#
# The symlinks travel apart because neither package tool can carry them:
# cargo-generate-rpm copies the file a link points at, and cargo-deb copies
# a linked directory's contents (Otto-MacTahoe-light links whole directories
# into Otto-MacTahoe). Either way the theme triples in size. The deb and rpm
# install the list with resources/icon-theme-links; the PKGBUILDs extract it
# into the package, so pacman owns the links.
#
# Running it again with the same pin does nothing.
set -euo pipefail

NAME=Otto-MacTahoe
THEMES=("$NAME" "$NAME-light" "$NAME-dark")
TAG=2026-09-10
SHA256=6330369e9e10a28cfc8da598ebf63a7204be705403519963ff84e1cb84719d35
REPO=https://github.com/vinceliuice/MacTahoe-icon-theme
URL="https://codeload.github.com/vinceliuice/MacTahoe-icon-theme/tar.gz/refs/tags/$TAG"

cd "$(dirname "$0")/../.."
outdir="${1:-target/icon-theme}"

if [ "$(cat "$outdir/.tag" 2>/dev/null)" = "$NAME-$TAG" ]; then
    exit 0
fi

tmpdir=$(mktemp -d)
trap 'rm -rf "$tmpdir"' EXIT

curl -fsSL --retry 3 --retry-delay 5 -o "$tmpdir/theme.tar.gz" "$URL"
echo "$SHA256  $tmpdir/theme.tar.gz" | sha256sum -c --quiet -
tar -xzf "$tmpdir/theme.tar.gz" -C "$tmpdir"
src="$tmpdir/MacTahoe-icon-theme-$TAG"

rm -rf "$outdir"
mkdir -p "$outdir"
outdir=$(cd "$outdir" && pwd)
# The installer only copies, seds and links, then runs gtk-update-icon-cache
# under `set -e` after each theme. That is missing from a minimal build
# environment (the Arch container in CI), which stops the installer after
# the first theme. The packages want no cache anyway — each distribution
# rebuilds icon caches when a package adds files under /usr/share/icons — so
# a no-op stands in for it everywhere.
#
# The installer also reads ambient variables (DESKTOP_SESSION, name, dest,
# bold, …), so it runs with an empty environment: the output must not depend
# on whose session built the package.
mkdir "$tmpdir/bin" "$tmpdir/home"
printf '#!/bin/sh\nexit 0\n' > "$tmpdir/bin/gtk-update-icon-cache"
chmod 755 "$tmpdir/bin/gtk-update-icon-cache"
if ! (cd "$src" && env -i PATH="$tmpdir/bin:/usr/bin:/bin" HOME="$tmpdir/home" \
        bash install.sh -n "$NAME" -d "$outdir") > "$tmpdir/install.log" 2>&1; then
    cat "$tmpdir/install.log" >&2
    echo "fetch-icon-theme: MacTahoe's install.sh failed" >&2
    exit 1
fi
for theme in "${THEMES[@]}"; do
    if [ ! -f "$outdir/$theme/index.theme" ]; then
        cat "$tmpdir/install.log" >&2
        echo "fetch-icon-theme: $theme was not installed" >&2
        exit 1
    fi
    rm -f "$outdir/$theme/icon-theme.cache"
    # The installer renames directories and links but leaves Name= at
    # upstream's "MacTahoe" in all three, which is what a theme picker shows.
    sed -i "s/^Name=.*/Name=$theme/" "$outdir/$theme/index.theme"
done
# Sorted and with fixed owner and time, so the same pin gives the same tar.
(
    cd "$outdir"
    find "${THEMES[@]}" -type l | LC_ALL=C sort > "$tmpdir/links"
    tar -cf - --no-recursion --owner=0 --group=0 --numeric-owner \
        --mtime=@0 -T "$tmpdir/links" | gzip -9n > "links-$TAG.tar.gz"
    xargs -d '\n' rm -f < "$tmpdir/links"
)
install -m644 "$src/COPYING" "$outdir/COPYING"
cat > "$outdir/SOURCE" <<EOF
Otto-MacTahoe, Otto-MacTahoe-light and Otto-MacTahoe-dark are the MacTahoe
icon theme by Vince Liu, licensed GPL-3.0 (see MacTahoe-COPYING), installed
under Otto's name by its own install.sh (-n $NAME). The only change is the
Name= line of each index.theme, set to the theme's directory name.

Corresponding source, including the SVG sources of the XCursor files under
each theme's cursors/ directory:
  $REPO/tree/$TAG
  $URL
  sha256 $SHA256
EOF
echo "$NAME-$TAG" > "$outdir/.tag"
