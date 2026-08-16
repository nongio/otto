#!/bin/bash
# Build both user guide and developer guide

SCRIPT_DIR="$(dirname "$0")"
DOCS_DIR="$SCRIPT_DIR/../docs"
OUTPUT_DIR="$SCRIPT_DIR/content"

# content/ only holds generated, gitignored files, so a fresh clone
# (e.g. CI) has no content/ directory at all.
mkdir -p "$OUTPUT_DIR"

# ============================================
# USER GUIDE (one Hugo page per doc, not concatenated)
# ============================================
rm -f "$OUTPUT_DIR"/*.md

USER_FILES=(
    "user/README.md"
    "user/getting-started.md"
    "user/desktop-tour.md"
    "user/window-management.md"
    "user/workspaces.md"
    "user/expose-and-switcher.md"
    "user/dock.md"
    "user/topbar.md"
    "user/dynamic-island.md"
    "user/keyboard-shortcuts.md"
    "user/gestures.md"
    "user/configuration.md"
    "user/display.md"
    "user/theming.md"
    "user/input.md"
    "user/audio.md"
    "user/power-management.md"
    "user/night-shift.md"
    "user/autostart.md"
    "user/clipboard.md"
    "user/lock-screen.md"
    "user/login-greeter.md"
    "user/screen-sharing.md"
    "user/remote-desktop.md"
    "user/troubleshooting.md"
)

echo "Building User Guide..."
for file in "${USER_FILES[@]}"; do
    filepath="$DOCS_DIR/$file"
    if [ ! -f "$filepath" ]; then
        echo "⚠ Warning: $file not found"
        continue
    fi

    base="$(basename "$file" .md)"
    slug="$(echo "$base" | tr '[:upper:]' '[:lower:]')"
    title="$(sed -n '1s/^# //p' "$filepath")"
    # README.md is the guide's landing page, served at "/"; everything
    # else gets its own top-level page at "/<slug>/".
    if [ "$slug" = "readme" ]; then
        outfile="$OUTPUT_DIR/_index.md"
    else
        outfile="$OUTPUT_DIR/$slug.md"
    fi

    {
        echo "---"
        echo "title: \"$title\""
        if [ "$slug" != "readme" ]; then
            echo 'layout: "doc"'
        fi
        echo "---"
        echo ""
        tail -n +2 "$filepath"
    } > "$outfile"
done

# docs/user/*.md link to sibling files (for GitHub rendering); each
# page is its own Hugo page here, so rewrite those into real page
# links instead of leaving the raw ".md" filename in the href.
perl -i -pe '
    s{\]\(README\.md\)}{](/)}gi;
    s{\]\(([a-zA-Z0-9_-]+)\.md#([a-zA-Z0-9_-]+)\)}{](/$1/#$2)}g;
    s{\]\(([a-zA-Z0-9_-]+)\.md\)}{](/$1/)}g;
' "$OUTPUT_DIR"/*.md

# ============================================
# DEVELOPER GUIDE
# ============================================
# Same shape as the user guide: one Hugo page per doc, so each page gets its
# own table of contents. Concatenating them into a single page produced one
# enormous TOC that was unusable once you started scrolling.
# These live under /developer/ rather than at the top level, so the guides
# keep separate URL namespaces and /developer/ stays a valid landing page.
DEV_OUT="$OUTPUT_DIR/developer"
rm -rf "$DEV_OUT"
mkdir -p "$DEV_OUT"

DEVELOPER_FILES=(
    "developer/README.md"
    "developer/project-structure.md"
    "developer/rendering.md"
    "developer/render_loop.md"
    "developer/wayland.md"
    "developer/drm_plane.md"
    "developer/dock-design.md"
    "developer/expose.md"
    "developer/window-move.md"
    "developer/foreign-toplevel.md"
    "developer/screenshare.md"
    "developer/color-scheme-setting.md"
    "developer/settings-dbus-api.md"
    "developer/rdp-virtual-output.md"
    "developer/otto-kit-roadmap.md"
    "developer/sc-layer-protocol-design.md"
    "developer/screenshot-plan.md"
    "developer/airplay-screenshare.md"
)

echo "Building Developer Guide..."
for file in "${DEVELOPER_FILES[@]}"; do
    filepath="$DOCS_DIR/$file"
    if [ ! -f "$filepath" ]; then
        echo "⚠ Warning: $file not found"
        continue
    fi

    base="$(basename "$file" .md)"
    slug="$(echo "$base" | tr '[:upper:]' '[:lower:]')"
    title="$(sed -n '1s/^# //p' "$filepath")"
    # README.md is the guide's landing page at "/developer/"; every other
    # doc gets its own page at "/developer/<slug>/".
    if [ "$slug" = "readme" ]; then
        outfile="$DEV_OUT/_index.md"
    else
        outfile="$DEV_OUT/$slug.md"
    fi

    {
        echo "---"
        echo "title: \"$title\""
        echo 'layout: "doc"'
        echo "---"
        echo ""
        tail -n +2 "$filepath"
    } > "$outfile"
done

# docs/developer/*.md link to sibling files, to diagrams, and out into the
# repo (../../specs, ../../protocols) so they render on GitHub. Rewrite each
# kind for the site: siblings become real page links, diagrams point at the
# static dir, and repo paths become GitHub links since they have no page here.
perl -i -pe '
    s{\]\(README\.md\)}{](/developer/)}gi;
    s{\]\(diagrams/([A-Za-z0-9_-]+\.svg)\)}{](/diagrams/$1)}g;
    s{\]\(\.\./\.\./([^)]*/)\)}{](https://github.com/nongio/otto/tree/main/$1)}g;
    s{\]\(\.\./\.\./([^)]+)\)}{](https://github.com/nongio/otto/blob/main/$1)}g;
    s{\]\(([A-Za-z0-9_-]+)\.md#([^)]+)\)}{"](/developer/" . lc($1) . "/#$2)"}ge;
    s{\]\(([A-Za-z0-9_-]+)\.md\)}{"](/developer/" . lc($1) . "/)"}ge;
' "$DEV_OUT"/*.md

# The diagrams are referenced as /diagrams/<name>.svg above; Hugo serves
# assets/ as the static dir, and canonifyURLs resolves the root-relative
# path against the real baseURL.
DIAGRAM_SRC="$DOCS_DIR/developer/diagrams"
if [ -d "$DIAGRAM_SRC" ]; then
    mkdir -p "$SCRIPT_DIR/assets/diagrams"
    cp "$DIAGRAM_SRC"/*.svg "$SCRIPT_DIR/assets/diagrams/" 2>/dev/null
fi

echo "✓ Built User Guide: $OUTPUT_DIR/_index.md"
echo "✓ Built Developer Guide: $DEV_OUT/ ($(ls "$DEV_OUT" | wc -l) pages)"
