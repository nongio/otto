#!/bin/bash
# Run otto in udev mode with scanout diagnostics.
# Logs to /tmp/otto.log for cross-session inspection.
cd "$(dirname "$0")"
RUST_LOG=warn,otto::scanout=debug target/release/otto --tty-udev 2>&1 | tee /tmp/otto.log
