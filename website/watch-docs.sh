#!/bin/bash
# Auto-rebuild docs when markdown files in docs/ change
# Uses polling instead of inotify (no dependencies needed)

SCRIPT_DIR="$(dirname "$0")"
DOCS_DIR="$SCRIPT_DIR/../docs"

echo "👁️  Watching $DOCS_DIR for changes..."
echo "Press Ctrl+C to stop"
echo ""

# Build once on start
./build-docs.sh
echo ""

# Get checksum of all markdown files
get_checksum() {
    find "$DOCS_DIR" -name "*.md" -type f -exec md5sum {} \; 2>/dev/null | sort | md5sum
}

LAST_CHECKSUM=$(get_checksum)

# Poll for changes every 2 seconds
while true; do
    sleep 2
    CURRENT_CHECKSUM=$(get_checksum)
    
    if [ "$CURRENT_CHECKSUM" != "$LAST_CHECKSUM" ]; then
        echo "🔄 Detected changes, rebuilding..."
        ./build-docs.sh
        echo ""
        LAST_CHECKSUM=$CURRENT_CHECKSUM
    fi
done
