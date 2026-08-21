#!/bin/bash
# Re-point xdg-desktop-portal at the currently running otto backend.
#
# Run this after restarting or reinstalling xdg-desktop-portal-otto inside a
# live session: the frontend caches the backend's properties from when it
# loaded the implementation, so a backend that came up later leaves it
# advertising AvailableCursorModes=0 and every screencast client fails with
# "Unavailable cursor mode 2".

set -e

GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m'

log_info() { echo -e "${GREEN}[INFO]${NC} $1"; }
log_warn() { echo -e "${YELLOW}[WARN]${NC} $1"; }

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
source "$SCRIPT_DIR/portal.sh"

export XDG_RUNTIME_DIR="${XDG_RUNTIME_DIR:-/run/user/$(id -u)}"
export XDG_CURRENT_DESKTOP="${XDG_CURRENT_DESKTOP:-otto}"
export XDG_SESSION_TYPE="${XDG_SESSION_TYPE:-wayland}"

if ! busctl --user list 2>/dev/null | grep -q "org.freedesktop.impl.portal.desktop.otto"; then
    log_warn "No otto portal backend on the bus — start it before refreshing the frontend"
fi

portal_frontend_reload
