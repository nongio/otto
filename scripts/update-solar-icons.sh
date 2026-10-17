#!/usr/bin/env bash
# Refresh the Solar icons Otto's own interface draws its glyphs with.
#
#   scripts/update-solar-icons.sh           # fetch the pinned release, rewrite the set
#   scripts/update-solar-icons.sh --check   # offline: the set matches the list
#
# Toolbar, menu and control glyphs in Otto's apps come from Solar (by 480
# Design, CC BY 4.0), not from the person's icon theme; app icons, file
# types and tray icons still come from the theme. Only the icons named in
# resources/icons/solar/icons.list are kept, and they are committed: this
# runs by hand when the list or the pinned release changes, never in a build.
#
# icons.list, one icon per line:
#   <otto name>  <solar name>  [style ...]     # styles default to "linear"
# e.g.
#   zoom-in-symbolic  magnifer-zoom-in  linear bold
# writes resources/icons/solar/linear/zoom-in-symbolic.svg and
# resources/icons/solar/bold/zoom-in-symbolic.svg.
#
# Styles: linear, bold, outline, broken, line-duotone, bold-duotone.
set -euo pipefail

VERSION=2.3.2
SHA256=af8941e3ab5443fea20c0c090a8d92a19d624203908ea025dd5bf1f73758a919
URL="https://registry.npmjs.org/@solar-icons/static/-/static-$VERSION.tgz"

cd "$(dirname "$0")/.."
dir=resources/icons/solar
list="$dir/icons.list"

# The entries of icons.list as "otto solar style" lines, one per style.
entries() {
    sed -e 's/#.*//' "$list" | while read -r name solar styles; do
        [ -n "$name" ] || continue
        if [ -z "$solar" ]; then
            echo "icons.list: '$name' names no Solar icon" >&2
            exit 1
        fi
        for style in ${styles:-linear}; do
            echo "$name $solar $style"
        done
    done
}

# A malformed line stops everything here, before any file is touched.
entries > /dev/null

if [ "${1:-}" = "--check" ]; then
    status=0
    wanted=$(entries | awk '{ print $3 "/" $1 ".svg" }' | sort)
    have=$(cd "$dir" && find . -mindepth 2 -name '*.svg' | sed 's|^\./||' | sort)
    missing=$(comm -23 <(echo "$wanted") <(echo "$have"))
    extra=$(comm -13 <(echo "$wanted") <(echo "$have"))
    if [ -n "$missing" ]; then
        echo "listed but not in $dir (run scripts/update-solar-icons.sh):" >&2
        echo "$missing" | sed 's/^/  /' >&2
        status=1
    fi
    if [ -n "$extra" ]; then
        echo "in $dir but not listed:" >&2
        echo "$extra" | sed 's/^/  /' >&2
        status=1
    fi
    if [ "$(cat "$dir/VERSION" 2>/dev/null)" != "$VERSION" ]; then
        echo "$dir/VERSION isn't $VERSION (run scripts/update-solar-icons.sh)" >&2
        status=1
    fi
    [ "$status" = 0 ] && echo "solar icons are current ($(echo "$wanted" | grep -c .) files)"
    exit "$status"
fi

tmpdir=$(mktemp -d)
trap 'rm -rf "$tmpdir"' EXIT

curl -fsSL --retry 3 --retry-delay 5 -o "$tmpdir/solar.tgz" "$URL"
echo "$SHA256  $tmpdir/solar.tgz" | sha256sum -c --quiet -
mkdir "$tmpdir/pkg"
tar -xzf "$tmpdir/solar.tgz" -C "$tmpdir/pkg" \
    package/LICENSE package/LICENSE-THIRD-PARTY package/dist/icons
icons="$tmpdir/pkg/package/dist/icons"

# Every entry must name a real icon before anything is replaced.
status=0
while read -r name solar style; do
    if [ ! -f "$icons/$style/$solar.svg" ]; then
        echo "icons.list: no Solar icon '$solar' in style '$style' (for $name)" >&2
        near=$(cd "$icons/linear" 2>/dev/null && ls | sed -n 's/\.svg$//p' \
            | grep -F -- "${solar%%-*}" | head -8 | tr '\n' ' ')
        [ -n "$near" ] && echo "  close: $near" >&2
        status=1
    fi
done < <(entries)
[ "$status" = 0 ] || exit 1

find "$dir" -mindepth 2 -name '*.svg' -delete
find "$dir" -mindepth 1 -type d -empty -delete
while read -r name solar style; do
    mkdir -p "$dir/$style"
    cp "$icons/$style/$solar.svg" "$dir/$style/$name.svg"
done < <(entries)

{
    echo "Icons in this folder are from the Solar icon set by 480 Design,"
    echo "https://www.figma.com/community/file/1166831539721848736,"
    echo "licensed under CC BY 4.0, https://creativecommons.org/licenses/by/4.0/."
    echo "Packaged as @solar-icons/static $VERSION (MIT), https://github.com/saoudi-h/solar-icons."
    echo "Files are renamed to the icon names Otto asks for; their drawing is unchanged."
    echo
    echo "--- LICENSE-THIRD-PARTY ---"
    cat "$tmpdir/pkg/package/LICENSE-THIRD-PARTY"
    echo
    echo "--- LICENSE ---"
    cat "$tmpdir/pkg/package/LICENSE"
} > "$dir/LICENSE"
echo "$VERSION" > "$dir/VERSION"

echo "wrote $(entries | wc -l) icons to $dir"
