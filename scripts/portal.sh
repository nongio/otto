#!/bin/bash

portal_setup() {
    # Ensure portal backend is built
    if [ ! -f "target/release/xdg-desktop-portal-otto" ]; then
        log_error "Portal backend not built in release mode!"
        log_info "Please run: cargo build -p xdg-desktop-portal-otto --release"
        exit 1
    fi

    # Start portal backend in background
    log_info "Starting xdg-desktop-portal-otto"
    PORTAL_LOG="$PWD/components/xdg-desktop-portal-otto/portal.log"
    mkdir -p "$(dirname "$PORTAL_LOG")"

    # Kill existing portal if running
    pkill -f xdg-desktop-portal-otto || true
    sleep 0.5

    # Start portal backend
    RUST_LOG=$LOG_LEVEL target/release/xdg-desktop-portal-otto > "$PORTAL_LOG" 2>&1 &
    PORTAL_PID=$!
    log_info "Portal backend started (PID: $PORTAL_PID, log: $PORTAL_LOG)"

    # Wait for the backend to claim its bus name
    for _ in $(seq 20); do
        if busctl --user list 2>/dev/null | grep -q "org.freedesktop.impl.portal.desktop.otto"; then
            break
        fi
        sleep 0.25
    done

    # Verify portal is running
    if ! busctl --user list | grep -q "org.freedesktop.impl.portal.desktop.otto"; then
        log_error "Portal backend failed to start!"
        cat "$PORTAL_LOG"
        exit 1
    fi
    log_info "Portal backend registered on D-Bus"
}

# xdg-desktop-portal reads the backend's properties once, when it loads the
# implementation, and only picks otto.portal at all when XDG_CURRENT_DESKTOP
# says "otto". A frontend that started before the backend — or before that env
# reached systemd --user — reports AvailableCursorModes=0 and rejects every
# SelectSources with "Unavailable cursor mode". Restart it once the backend is
# up, then check that the values actually propagated.
portal_frontend_reload() {
    command -v systemctl >/dev/null 2>&1 || return 0

    systemctl --user set-environment \
        XDG_CURRENT_DESKTOP="${XDG_CURRENT_DESKTOP:-otto}" \
        XDG_SESSION_TYPE="${XDG_SESSION_TYPE:-wayland}" \
        XDG_RUNTIME_DIR="$XDG_RUNTIME_DIR" \
        WAYLAND_DISPLAY="$WAYLAND_DISPLAY" 2>/dev/null || true

    if ! systemctl --user restart xdg-desktop-portal.service 2>/dev/null; then
        log_warn "Could not restart xdg-desktop-portal user service"
        return 0
    fi
    log_info "Restarted xdg-desktop-portal user service"

    local modes=""
    for _ in $(seq 20); do
        modes=$(busctl --user get-property org.freedesktop.portal.Desktop \
            /org/freedesktop/portal/desktop org.freedesktop.portal.ScreenCast \
            AvailableCursorModes 2>/dev/null | awk '{print $2}')
        [ -n "$modes" ] && [ "$modes" != "0" ] && break
        sleep 0.25
    done

    if [ -n "$modes" ] && [ "$modes" != "0" ]; then
        log_info "Portal frontend sees AvailableCursorModes=$modes"
    else
        log_warn "Portal frontend still reports AvailableCursorModes=${modes:-unknown}"
        log_warn "Screencast clients will fail with 'Unavailable cursor mode'"
    fi
}
