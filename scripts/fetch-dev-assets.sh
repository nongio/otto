#!/usr/bin/env bash
# Stage the assets a packaged Otto installs, for running it from this
# checkout (`cargo run -- --winit`) without installing anything.
#
#   scripts/fetch-dev-assets.sh
#
# Leaves under target/share, which Otto picks up on its own when it runs
# from the checkout (src/checkout.rs):
#   icons/Otto-MacTahoe*   the icon theme, links included
#   fonts/Inter-*.ttf      Inter, the interface font
#   fonts.conf             the system's fontconfig, plus fonts/
#
# The wallpaper needs nothing: it is read from resources/ directly.
#
# Running it again does nothing that is already done.
set -euo pipefail

INTER_TAG=4.1
INTER_SHA256=9883fdd4a49d4fb66bd8177ba6625ef9a64aa45899767dde3d36aa425756b11e
INTER_URL="https://github.com/rsms/inter/releases/download/v$INTER_TAG/Inter-$INTER_TAG.zip"

cd "$(dirname "$0")/.."
share="$(pwd)/target/share"

# The packaging script leaves the theme's symlinks in a tarball, for the
# package tools; here they go straight back where they belong.
scripts/packaging/fetch-icon-theme.sh "$share/icons"
tar -xzf "$share"/icons/links-*.tar.gz -C "$share/icons"

fonts="$share/fonts"
if [ "$(cat "$fonts/.tag" 2>/dev/null)" != "Inter-$INTER_TAG" ]; then
    tmpdir=$(mktemp -d)
    trap 'rm -rf "$tmpdir"' EXIT
    curl -fsSL --retry 3 --retry-delay 5 -o "$tmpdir/inter.zip" "$INTER_URL"
    echo "$INTER_SHA256  $tmpdir/inter.zip" | sha256sum -c --quiet -
    rm -rf "$fonts"
    mkdir -p "$fonts"
    unzip -q -j "$tmpdir/inter.zip" 'extras/ttf/Inter-*.ttf' LICENSE.txt -d "$fonts"
    echo "Inter-$INTER_TAG" > "$fonts/.tag"
fi

cat > "$share/fonts.conf" <<EOF
<?xml version="1.0"?>
<!DOCTYPE fontconfig SYSTEM "urn:fontconfig:fonts.dtd">
<fontconfig>
  <include ignore_missing="yes">/etc/fonts/fonts.conf</include>
  <dir>$fonts</dir>
</fontconfig>
EOF

echo "Staged in $share — run Otto from the checkout to use it."
