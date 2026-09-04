#!/usr/bin/env bash
# Assemble the Arch release tarball: the release binaries plus every data file
# the binary PKGBUILDs install out of it.
#
#   scripts/packaging/make-arch-tarball.sh [output-dir]
#
# CI calls this, and so does scripts/packaging/test-installers.sh. That is the
# point of it being a script rather than an inline CI step: PKGBUILD and
# PKGBUILD-nightly-bin install files *from this tarball*, so a file listed in
# one and missing from the other breaks `makepkg` for every Arch user — and
# nothing catches it until someone tries to install a published release.
# Sharing one list means the packaging test exercises what CI actually ships.
#
# Requires target/release to be populated (a release build, or CI's downloaded
# artifacts).
set -euo pipefail

cd "$(dirname "$0")/../.."
outdir="${1:-$PWD}"
mkdir -p "$outdir"
outdir=$(cd "$outdir" && pwd)

# Read the version out of [workspace.package]. Not `head -1` on the first
# `version = ` line: the crate inherits with `version.workspace = true`, so
# the first such line in the file belongs to a [dependencies.*] table and
# names that dependency.
PKGVER=$(sed -n '/^\[workspace.package\]/,/^\[/p' Cargo.toml | sed -n 's/^version = "\(.*\)"/\1/p' | head -1)
if [ -z "$PKGVER" ]; then
    echo "no [workspace.package] version found in Cargo.toml" >&2
    exit 1
fi

PKGDIR="otto-${PKGVER}"
TARBALL="$outdir/otto-${PKGVER}-x86_64.tar.gz"
tmpdir=$(mktemp -d)
trap 'rm -rf "$tmpdir"' EXIT

for b in otto otto-bar otto-islands otto-lock otto-greeter otto-rdp \
         otto-settings otto-files otto-launcher otto-quickview \
         xdg-desktop-portal-otto; do
    install -Dm755 "target/release/$b" "$tmpdir/$PKGDIR/target/release/$b"
done

install -m644 LICENSE                  "$tmpdir/$PKGDIR/LICENSE"
install -m644 README.md                "$tmpdir/$PKGDIR/README.md"
install -m644 otto_config.example.toml "$tmpdir/$PKGDIR/otto_config.example.toml"

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
install -Dm644 components/otto-lock/otto-lock.pam \
    "$tmpdir/$PKGDIR/components/otto-lock/otto-lock.pam"

install -m644 PKGBUILD-git         "$tmpdir/$PKGDIR/PKGBUILD-git"
install -m644 PKGBUILD-nightly-bin "$tmpdir/$PKGDIR/PKGBUILD-nightly-bin"

# VERSION file for PKGBUILD-nightly-bin's pkgver()
echo "${PKGVER}.r$(git rev-list --count HEAD).$(git rev-parse --short HEAD)" \
    > "$tmpdir/$PKGDIR/VERSION"

tar -czf "$TARBALL" -C "$tmpdir" "$PKGDIR"
# Fixed-name copy for the nightly release
cp "$TARBALL" "$outdir/otto-nightly-x86_64.tar.gz"

echo "$TARBALL"
