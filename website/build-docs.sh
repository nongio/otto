#!/bin/bash
# Build both user guide and developer guide

SCRIPT_DIR="$(dirname "$0")"
DOCS_DIR="$SCRIPT_DIR/../docs"
OUTPUT_DIR="$SCRIPT_DIR/content"

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
        if [ "$slug" = "readme" ]; then
            # The home layout's hero CSS only special-cases the single
            # paragraph immediately after <h1> (h1 + p); README.md's
            # intro is two paragraphs, and the second one falls out of
            # the hero's fixed-height box and collides with the TOC
            # sidebar once it appears. Merge the intro into one
            # paragraph so it all gets the hero treatment.
            tail -n +2 "$filepath" | perl -0777 -pe 's/\n\n/ /'
        else
            tail -n +2 "$filepath"
        fi
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
cat > "$OUTPUT_DIR/developer.md" << 'INTRO'
---
title: "Otto Developer Guide"
layout: "doc"
---

INTRO

DEVELOPER_FILES=(
    "developer/intro.md"
    "developer/project-structure.md"
    "developer/rendering.md"
    "developer/render_loop.md"
    "developer/wayland.md"
    "developer/screenshare.md"
    # "developer/screenshot-plan.md"
    "developer/dock-design.md"
    # "developer/expose.md"
    # "developer/layer-shell.md"
    
    
    # "developer/drm_plane.md"
    # "developer/foreign-toplevel.md"
    # "developer/keyboard_mapping.md"
    # "developer/window-move.md"
    # "developer/sc-layer-protocol-design.md"
    "developer/credits.md"
)

echo "Building Developer Guide..."
for file in "${DEVELOPER_FILES[@]}"; do
    filepath="$DOCS_DIR/$file"
    if [ -f "$filepath" ]; then
        echo "" >> "$OUTPUT_DIR/developer.md"
        cat "$filepath" >> "$OUTPUT_DIR/developer.md"
        echo "" >> "$OUTPUT_DIR/developer.md"
    else
        echo "⚠ Warning: $file not found"
    fi
done

echo "✓ Built User Guide: $OUTPUT_DIR/_index.md"
echo "✓ Built Developer Guide: $OUTPUT_DIR/developer.md"
