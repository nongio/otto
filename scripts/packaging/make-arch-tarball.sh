#!/usr/bin/env bash
# Assemble the Arch release tarball: the release binaries plus every data file
# the binary PKGBUILDs install out of it.
#
#   scripts/packaging/make-arch-tarball.sh [output-dir]      # default: dist/
#   scripts/packaging/make-arch-tarball.sh --stand-in [output-dir]
#
# CI calls this, and so does scripts/packaging/test-installers.sh. That is the
# point of it being a script rather than an inline CI step: PKGBUILD and
# PKGBUILD-nightly-bin install files *from this tarball*, so a file listed in
# one and missing from the other breaks `makepkg` for every Arch user — and
# nothing catches it until someone tries to install a published release.
# Sharing one list means the packaging test exercises what CI actually ships.
#
# Writes three files to the output directory: the versioned tarball a release
# publishes, the fixed-name copy the nightly publishes, and the
# PKGBUILD-nightly-bin pinned to that copy.
#
# Requires target/release to be populated (a release build, or CI's downloaded
# artifacts), and the full git history: the nightly pkgver counts commits.
# --stand-in writes a shell script in place of each binary that is missing,
# for checking the packaging itself — that every file the PKGBUILDs install
# is in the tarball — without a release build.
set -euo pipefail

cd "$(dirname "$0")/../.."
stand_in=0
if [ "${1:-}" = --stand-in ]; then stand_in=1; shift; fi
outdir="${1:-dist}"
mkdir -p "$outdir"
outdir=$(cd "$outdir" && pwd)

BINARIES=(otto otto-bar otto-islands otto-lock otto-greeter otto-rdp
          otto-settings otto-files otto-launcher otto-emoji otto-peek
          otto-media-worker otto-msg otto-agents xdg-desktop-portal-otto)

# The workspace version names the tarball and its top directory; the
# nightly pkgver goes into the VERSION file below. Both come from the one
# script that spells versions for every package format.
PKGVER=$(scripts/packaging/version.sh workspace)
NIGHTLY_PKGVER=$(OTTO_NIGHTLY=1 scripts/packaging/version.sh arch)

PKGDIR="otto-${PKGVER}"
TARBALL="$outdir/otto-${PKGVER}-x86_64.tar.gz"
tmpdir=$(mktemp -d)
trap 'rm -rf "$tmpdir"' EXIT

for b in "${BINARIES[@]}"; do
    if [ "$stand_in" = 1 ] && [ ! -e "target/release/$b" ]; then
        # Into the staging directory only: a stub left in target/release
        # would be packaged by a later build that did not rebuild it.
        mkdir -p "$tmpdir/$PKGDIR/target/release"
        printf '#!/bin/sh\necho "stand-in for %s"\n' "$b" > "$tmpdir/$PKGDIR/target/release/$b"
        chmod 755 "$tmpdir/$PKGDIR/target/release/$b"
        continue
    fi
    install -Dm755 "target/release/$b" "$tmpdir/$PKGDIR/target/release/$b"
done

install -m644 LICENSE                  "$tmpdir/$PKGDIR/LICENSE"
install -m644 README.md                "$tmpdir/$PKGDIR/README.md"
install -m644 otto_config.example.toml "$tmpdir/$PKGDIR/otto_config.example.toml"
install -Dm755 resources/bin/otto-look "$tmpdir/$PKGDIR/resources/bin/otto-look"

# Desktop entries. otto-trash.desktop is the Trash window — otto-files behind
# its own entry, so it gets its own icon in the dock and the applications
# list. All three PKGBUILDs install it; leaving it out of the tarball fails
# package() with "cannot stat".
for d in otto.desktop otto-files.desktop otto-settings.desktop otto-trash.desktop; do
    install -Dm644 "resources/$d" "$tmpdir/$PKGDIR/resources/$d"
done

for px in 16 24 32 48 64 128 256 512; do
    install -Dm644 "components/otto-files/resources/icons/hicolor/${px}x${px}/apps/otto-files.png" \
        "$tmpdir/$PKGDIR/components/otto-files/resources/icons/hicolor/${px}x${px}/apps/otto-files.png"
done
install -Dm644 components/otto-files/resources/icons/hicolor/scalable/apps/otto-files.svg \
    "$tmpdir/$PKGDIR/components/otto-files/resources/icons/hicolor/scalable/apps/otto-files.svg"

for f in otto.portal \
         org.freedesktop.impl.portal.desktop.otto.service \
         xdg-desktop-portal-otto.service \
         portals.conf.example; do
    install -Dm644 "components/xdg-desktop-portal-otto/$f" \
        "$tmpdir/$PKGDIR/components/xdg-desktop-portal-otto/$f"
done
install -Dm644 components/otto-agents/otto-agents.service "$tmpdir/$PKGDIR/components/otto-agents/otto-agents.service"
install -Dm644 components/otto-lock/otto-lock.pam \
    "$tmpdir/$PKGDIR/components/otto-lock/otto-lock.pam"

# The agent skills, as a tree: PKGBUILD and PKGBUILD-nightly-bin install
# whatever is under resources/plugins/otto, so the tarball has to carry all of
# it, modes included — the example Files script is executable.
while IFS= read -r f; do
    if [ -x "$f" ]; then m=755; else m=644; fi
    install -D -m$m "$f" "$tmpdir/$PKGDIR/$f"
done < <(find resources/plugins/otto -type f)

install -m644 PKGBUILD-git "$tmpdir/$PKGDIR/PKGBUILD-git"

# Which build this is, as the nightly package's pkgver. PKGBUILD-nightly-bin
# is pinned to a tarball by make-nightly-pkgbuild.sh, which reads this file;
# the PKGBUILD itself checks it against its own pkgver in prepare().
echo "$NIGHTLY_PKGVER" > "$tmpdir/$PKGDIR/VERSION"

tar -czf "$TARBALL" -C "$tmpdir" "$PKGDIR"
# Fixed-name copy for the nightly release, and the PKGBUILD pinned to it.
cp "$TARBALL" "$outdir/otto-nightly-x86_64.tar.gz"
scripts/packaging/make-nightly-pkgbuild.sh "$outdir/otto-nightly-x86_64.tar.gz" "$outdir" >/dev/null

echo "$TARBALL"
