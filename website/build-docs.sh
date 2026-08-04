#!/bin/bash
# Build both user guide and developer guide

SCRIPT_DIR="$(dirname "$0")"
DOCS_DIR="$SCRIPT_DIR/../docs"
OUTPUT_DIR="$SCRIPT_DIR/content"

# ============================================
# USER GUIDE
# ============================================
cat > "$OUTPUT_DIR/_index.md" << 'INTRO'
---
title: "Otto Compositor - User Guide"
---

INTRO

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
    if [ -f "$filepath" ]; then
        base="$(basename "$file" .md)"
        slug="$(echo "$base" | tr '[:upper:]' '[:lower:]')"
        {
            echo ""
            echo "<a id=\"$slug\"></a>"
            echo ""
            cat "$filepath"
            echo ""
        } >> "$OUTPUT_DIR/_index.md"
    else
        echo "⚠ Warning: $file not found"
    fi
done

# docs/user/*.md link to sibling files (for GitHub rendering), but the
# user guide concatenates them into one long page, so those links need
# to become in-page anchors instead. Top-level file.md links become
# #<basename> (anchored above via the loop). Links into a specific
# subheading of another file need an explicit anchor too, since
# goldmark's auto heading IDs aren't stable once headings collide
# across the merged files.
perl -i -pe '
    s{^(### Getting an app to export its menu)}{<a id="getting-an-app-to-export-its-menu"></a>\n\n$1};
    s{^(## Virtual outputs)}{<a id="virtual-outputs"></a>\n\n$1};
    s{^(## Portal setup)}{<a id="portal-setup"></a>\n\n$1};
    s{^(## Always-on keys)}{<a id="always-on-keys"></a>\n\n$1};
    s{^(### Choosing which monitor)}{<a id="choosing-which-monitor"></a>\n\n$1};
' "$OUTPUT_DIR/_index.md"

perl -i -pe '
    s{\]\(([a-zA-Z0-9_-]+)\.md#([a-zA-Z0-9_-]+)\)}{](#$2)}g;
    s{\]\(([a-zA-Z0-9_-]+)\.md\)}{](#\L$1)}g;
' "$OUTPUT_DIR/_index.md"

# ============================================
# DEVELOPER GUIDE
# ============================================
cat > "$OUTPUT_DIR/developer.md" << 'INTRO'
---
title: "Otto Developer Guide"
layout: "developer"
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
