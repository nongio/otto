#!/usr/bin/env bash
# Stage the PDFium library Studio fills and marks up PDFs with.
#
#   scripts/packaging/fetch-pdfium.sh [output-dir]     # default: target/pdfium
#
# PDFium (Chrome's PDF engine) isn't packaged by Arch, Debian or Fedora, so
# the packages carry a prebuilt one from bblanchon/pdfium-binaries, installed
# as /usr/lib/otto/libpdfium.so where otto-pdf looks for it. Without it
# Studio still shows PDFs; only filling and marking them up is missing.
# build-deb.sh, build-rpm.sh, make-arch-tarball.sh and PKGBUILD-git all call
# this script, so the pinned release below is the one place to bump it.
#
# Leaves in the output directory:
#   libpdfium.so        the library
#   LICENSE             PDFium's licence (BSD-3-Clause / Apache-2.0)
#   licenses/           the licences of what it is built from
#   VERSION             the Chromium build it is
#
# Running it again with the same pin does nothing.
set -euo pipefail

TAG=chromium/8086
SHA256=588577cf52dabc1a444988bac841920df54cc2f141801424de97ab04f4fbb935
URL="https://github.com/bblanchon/pdfium-binaries/releases/download/$TAG/pdfium-linux-x64.tgz"

cd "$(dirname "$0")/../.."
outdir="${1:-target/pdfium}"

if [ "$(cat "$outdir/.tag" 2>/dev/null)" = "$TAG" ]; then
    exit 0
fi

tmpdir=$(mktemp -d)
trap 'rm -rf "$tmpdir"' EXIT

curl -fsSL --retry 3 --retry-delay 5 -o "$tmpdir/pdfium.tgz" "$URL"
echo "$SHA256  $tmpdir/pdfium.tgz" | sha256sum -c --quiet -
tar -xzf "$tmpdir/pdfium.tgz" -C "$tmpdir"

rm -rf "$outdir"
mkdir -p "$outdir"
cp "$tmpdir/lib/libpdfium.so" "$tmpdir/LICENSE" "$tmpdir/VERSION" "$outdir/"
cp -r "$tmpdir/licenses" "$outdir/"
echo "$TAG" > "$outdir/.tag"
