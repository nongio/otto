#!/bin/bash
# ScreenComposer Session Startup Script
# Sets up D-Bus, environment variables, and necessary services

set -e

# Parse command line arguments
DEBUG_MODE=false
if [ "$1" = "--debug" ]; then
    DEBUG_MODE=true
    shift
fi

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

log_info() {
    echo -e "${GREEN}[INFO]${NC} $1"
}

log_warn() {
    echo -e "${YELLOW}[WARN]${NC} $1"
}

log_error() {
    echo -e "${RED}[ERROR]${NC} $1"
}

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
source "$SCRIPT_DIR/wifi.sh"
source "$SCRIPT_DIR/kwallet.sh"
source "$SCRIPT_DIR/dbus.sh"
source "$SCRIPT_DIR/pipewire.sh"
source "$SCRIPT_DIR/portal.sh"

# Export essential environment variables
export XDG_RUNTIME_DIR="${XDG_RUNTIME_DIR:-/run/user/$(id -u)}"
export XDG_SESSION_TYPE="wayland"
export XDG_SESSION_CLASS="user"
# Ensure XDG Desktop Portal picks the Otto backend (UseIn=otto).
export XDG_CURRENT_DESKTOP="otto"
export XDG_CONFIG_HOME="${XDG_CONFIG_HOME:-$HOME/.config}"

# Wayland display will be set by compositor
export WAYLAND_DISPLAY="${WAYLAND_DISPLAY:-wayland-0}"

# Ensure systemd user services (including xdg-desktop-portal) see the same session env.
if command -v systemctl >/dev/null 2>&1; then
    systemctl --user import-environment \
        XDG_RUNTIME_DIR XDG_SESSION_TYPE XDG_CURRENT_DESKTOP XDG_CONFIG_HOME WAYLAND_DISPLAY || true
fi

setup_dbus_session

log_info "Environment setup:"
log_info "  XDG_RUNTIME_DIR=$XDG_RUNTIME_DIR"
log_info "  DBUS_SESSION_BUS_ADDRESS=$DBUS_SESSION_BUS_ADDRESS"
log_info "  XDG_SESSION_TYPE=$XDG_SESSION_TYPE"
log_info ""
log_info "To run commands in this session from another terminal:"
log_info "  source $DBUS_ENV_FILE"

# Create runtime directory if it doesn't exist
if [ ! -d "$XDG_RUNTIME_DIR" ]; then
    log_warn "XDG_RUNTIME_DIR doesn't exist, creating it"
    mkdir -p "$XDG_RUNTIME_DIR"
    chmod 700 "$XDG_RUNTIME_DIR"
fi

start_kwallet_service

LOG_LEVEL="info"
if [ "$DEBUG_MODE" = true ]; then
    LOG_LEVEL="debug"
    log_info "Debug mode enabled - using RUST_LOG=debug"
fi

pipewire_setup

wifi_autoconnect

# Ensure compositor is built in release mode
if [ ! -f "target/release/otto" ]; then
    log_error "Compositor not built in release mode!"
    log_info "Please run: cargo build --release"
    exit 1
fi

log_info "Starting Otto Compositor udev backend, RUST_LOG=$LOG_LEVEL"
COMPOSITOR_LOG="$PWD/otto.log"

if [ "$EUID" -ne 0 ] && [ -z "$LIBSEAT_BACKEND" ]; then
    log_warn "Running DRM backend without root - you may need libseat or run with sudo"
fi

# Start compositor in background first
RUST_LOG=$LOG_LEVEL target/release/otto --tty-udev 2> "$COMPOSITOR_LOG" &
COMPOSITOR_PID=$!
log_info "Compositor started in background PID: $COMPOSITOR_PID"

# Wait for compositor to create Wayland socket and D-Bus service
log_info "Waiting for compositor to initialize..."
WAIT_COUNT=0
while [ $WAIT_COUNT -lt 30 ]; do
    # Check if compositor process is still running
    if ! kill -0 $COMPOSITOR_PID 2>/dev/null; then
        log_error "Compositor process died during startup!"
        cat "$COMPOSITOR_LOG"
        exit 1
    fi
    
    # Check if D-Bus service is available
    if busctl --user list 2>/dev/null | grep -q "org.otto.ScreenCast"; then
        log_info "Compositor D-Bus service is ready"
        break
    fi
    
    sleep 0.5
    WAIT_COUNT=$((WAIT_COUNT + 1))
done

if [ $WAIT_COUNT -eq 30 ]; then
    log_error "Timeout waiting for compositor to start"
    log_info "Last 20 lines of compositor log:"
    tail -20 "$COMPOSITOR_LOG"
    kill $COMPOSITOR_PID 2>/dev/null || true
    exit 1
fi

# Now start portal after compositor is ready
portal_setup

# Force frontend portal to reload with the current session environment.
if command -v systemctl >/dev/null 2>&1; then
    if systemctl --user restart xdg-desktop-portal.service 2>/dev/null; then
        log_info "Restarted xdg-desktop-portal user service"
    else
        log_warn "Could not restart xdg-desktop-portal user service"
    fi
fi

# Bring compositor to foreground and tail its log
log_info "Compositor initialized successfully, following log..."
tail -f "$COMPOSITOR_LOG" &
TAIL_PID=$!

# Wait for compositor process
wait $COMPOSITOR_PID
COMPOSITOR_EXIT=$?

# Stop tail when compositor exits
kill $TAIL_PID 2>/dev/null || true

# RUST_LOG=info cargo run --release -- --tty-udev 2>&1 | tee "$COMPOSITOR_LOG"    ;;
# esac


# Clean up D-Bus session file
if [ -f "$DBUS_ENV_FILE" ]; then
    rm -f "$DBUS_ENV_FILE"
    log_info "Cleaned up D-Bus session file"
fi

# Stop D-Bus session if we started it
if [ -n "$DBUS_SESSION_BUS_PID" ]; then
    kill $DBUS_SESSION_BUS_PID 2>/dev/null || true
    log_info "Stopped D-Bus session"
fi

# Cleanup on exit
log_info "Compositor exited with code $COMPOSITOR_EXIT, cleaning up..."
kill $PORTAL_PID 2>/dev/null || true
log_info "Session ended"

exit $COMPOSITOR_EXIT
