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
    "user/intro.md"
    "user/configuration.md"
    "user/credits.md"
    # Add user-focused docs here
)

echo "Building User Guide..."
for file in "${USER_FILES[@]}"; do
    filepath="$DOCS_DIR/$file"
    if [ -f "$filepath" ]; then
        echo "" >> "$OUTPUT_DIR/_index.md"
        cat "$filepath" >> "$OUTPUT_DIR/_index.md"
        echo "" >> "$OUTPUT_DIR/_index.md"
    else
        echo "⚠ Warning: $file not found"
    fi
done

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
