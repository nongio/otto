#!/bin/bash
# Development server with auto-rebuild

cd "$(dirname "$0")"

# Run watch script in background
./watch-docs.sh &
WATCH_PID=$!

# Run hugo server
hugo server

# Cleanup: kill watch script when hugo server stops
kill $WATCH_PID 2>/dev/null
