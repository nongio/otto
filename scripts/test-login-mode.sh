#!/usr/bin/env bash
#
# Test harness for Otto's login mode (`otto --login`) and the otto-greeter
# client. See specs/login-mode.md.
#
# Tests are grouped by what they need from the environment, because most of
# this feature can be exercised without root or a spare VT:
#
#   check    static checks + unit tests            nothing
#   ipc      greetd wire protocol conversation     nothing
#   mock     greeter UI, built-in mock backend     a Wayland session
#   greeter  greeter UI against a fake greetd      a Wayland session
#   nested   full login mode inside a window       a Wayland session
#   tty      full login mode on the console        a free VT, takes the screen
#   greetd   generate a real greetd config         root to install
#
# Run `check` and `ipc` anywhere; run `greeter` for the real thing minus greetd.

set -euo pipefail

cd "$(dirname "$0")/.."
ROOT="$PWD"

SOCK="${SOCK:-/tmp/otto-fake-greetd.sock}"
PASSWORD="${FAKE_GREETD_PASSWORD:-otto}"
SCENARIO="${FAKE_GREETD_SCENARIO:-fingerprint}"
LOG_DIR="${LOG_DIR:-/tmp/otto-login-test}"

if [[ -t 1 ]]; then
    BOLD=$'\e[1m'; RED=$'\e[31m'; GREEN=$'\e[32m'; YELLOW=$'\e[33m'; DIM=$'\e[2m'; OFF=$'\e[0m'
else
    BOLD=''; RED=''; GREEN=''; YELLOW=''; DIM=''; OFF=''
fi

step() { echo; echo "${BOLD}==> $*${OFF}"; }
ok()   { echo "  ${GREEN}ok${OFF}   $*"; }
warn() { echo "  ${YELLOW}warn${OFF} $*"; }
fail() { echo "  ${RED}FAIL${OFF} $*"; return 1; }
note() { echo "  ${DIM}$*${OFF}"; }

PIDS=()
cleanup() {
    for pid in "${PIDS[@]:-}"; do
        [[ -n "$pid" ]] && kill "$pid" 2>/dev/null || true
    done
    rm -f "$SOCK"
}
trap cleanup EXIT

have_display() { [[ -n "${WAYLAND_DISPLAY:-}" ]]; }

require_display() {
    if ! have_display; then
        echo "${RED}This test needs a running Wayland session.${OFF}"
        echo "WAYLAND_DISPLAY is unset — start Otto (or any layer-shell compositor) first,"
        echo "or use '$0 tty' to run login mode directly on the console."
        exit 1
    fi
}

start_fake_greetd() {
    rm -f "$SOCK"
    mkdir -p "$LOG_DIR"
    FAKE_GREETD_PASSWORD="$PASSWORD" FAKE_GREETD_SCENARIO="$SCENARIO" \
        "$ROOT/target/debug/examples/fake_greetd" "$SOCK" &> "$LOG_DIR/fake-greetd.log" &
    PIDS+=($!)

    for _ in $(seq 1 50); do
        [[ -S "$SOCK" ]] && return 0
        sleep 0.1
    done
    fail "fake greetd did not create $SOCK (see $LOG_DIR/fake-greetd.log)"
}

# ---------------------------------------------------------------- check ----

cmd_check() {
    step "Formatting"
    if cargo fmt --all -- --check; then
        ok "cargo fmt clean"
    else
        fail "run 'cargo fmt --all'"
    fi

    step "Clippy"
    cargo clippy -p otto-greeter --all-targets -- -D warnings
    ok "otto-greeter clean"
    cargo clippy --features default -- -D warnings
    ok "compositor clean"

    step "Unit tests"
    cargo test -p otto-greeter
    ok "greeter tests pass"

    step "Build"
    cargo build --release
    cargo build --release -p otto-greeter
    cargo build -p otto-greeter --example fake_greetd
    ok "binaries built"

    step "CLI wiring"
    if ./target/release/otto --help | grep -q -- "--login"; then
        ok "--login is advertised in --help"
    else
        fail "--login missing from --help"
    fi
    # An unknown flag must still be rejected — a too-permissive arg parser
    # would silently swallow typos like --loginn. Capture first rather than
    # piping: otto exits non-zero here, and pipefail would mask grep's result.
    local rejected
    rejected="$(./target/release/otto --loginn 2>&1 || true)"
    if grep -q "Unknown argument" <<<"$rejected"; then
        ok "unknown flags still rejected"
    else
        fail "argument validation is too permissive"
    fi
}

# ------------------------------------------------------------------ ipc ----

# Drive the fake greetd through a full conversation and assert each reply.
# This covers the wire format end to end: native-endian u32 length prefix plus
# JSON body, in both directions.
cmd_ipc() {
    if ! command -v python3 >/dev/null; then
        warn "python3 not found — running the in-process framing test only"
        cargo test -p otto-greeter greetd::
        return
    fi

    step "Building fake greetd"
    cargo build -p otto-greeter --example fake_greetd
    ok "built"

    step "Driving a full greetd conversation"
    start_fake_greetd

    SOCK="$SOCK" PASSWORD="$PASSWORD" python3 - <<'PY'
import json, os, socket, struct, sys

sock = socket.socket(socket.AF_UNIX)
sock.connect(os.environ["SOCK"])
password = os.environ["PASSWORD"]
failures = []

def roundtrip(msg):
    body = json.dumps(msg).encode()
    sock.sendall(struct.pack("=I", len(body)) + body)
    (length,) = struct.unpack("=I", sock.recv(4))
    return json.loads(sock.recv(length))

def expect(label, got, **fields):
    bad = {k: (v, got.get(k)) for k, v in fields.items() if got.get(k) != v}
    if bad:
        failures.append(f"{label}: expected {fields}, got {got}")
        print(f"  \033[31mFAIL\033[0m {label}: {got}")
    else:
        print(f"  \033[32mok\033[0m   {label}")

# An info message (as pam_fprintd sends) must precede the password prompt and
# expects a null acknowledgement rather than user input.
expect("info message before the prompt",
       roundtrip({"type": "create_session", "username": "tester"}),
       type="auth_message", auth_message_type="info")
expect("password prompt after acknowledging",
       roundtrip({"type": "post_auth_message_response", "response": None}),
       type="auth_message", auth_message_type="secret")
expect("wrong password rejected",
       roundtrip({"type": "post_auth_message_response", "response": "wrong"}),
       type="error", error_type="auth_error")

# After a rejection the session must be cancelled before retrying.
expect("session cancels", roundtrip({"type": "cancel_session"}), type="success")
expect("retry reaches the info message again",
       roundtrip({"type": "create_session", "username": "tester"}),
       type="auth_message", auth_message_type="info")
expect("prompt again", roundtrip({"type": "post_auth_message_response", "response": None}),
       type="auth_message", auth_message_type="secret")
expect("correct password accepted",
       roundtrip({"type": "post_auth_message_response", "response": password}),
       type="success")
expect("session starts",
       roundtrip({"type": "start_session", "cmd": ["otto"], "env": []}),
       type="success")

# Empty usernames must not open a session.
expect("session cancels", roundtrip({"type": "cancel_session"}), type="success")
expect("empty username rejected",
       roundtrip({"type": "create_session", "username": ""}),
       type="error", error_type="auth_error")

sys.exit(1 if failures else 0)
PY
    ok "conversation matches greetd-ipc(7)"
}

# --------------------------------------------------------------- client ----

cmd_mock() {
    require_display
    step "Greeter with the built-in mock backend"
    note "password: otto   ·   Esc resets   ·   Tab cycles sessions"
    note "GREETD_SOCK is deliberately unset"
    cargo build -p otto-greeter
    env -u GREETD_SOCK ./target/debug/otto-greeter
}

cmd_greeter() {
    require_display
    step "Greeter against a fake greetd"
    cargo build -p otto-greeter --example fake_greetd
    cargo build -p otto-greeter
    start_fake_greetd
    note "scenario: $SCENARIO   ·   password: $PASSWORD"
    note "daemon log: $LOG_DIR/fake-greetd.log"
    note "this exercises the real IPC path, not the mock backend"
    echo
    GREETD_SOCK="$SOCK" ./target/debug/otto-greeter
    echo
    step "Daemon transcript"
    cat "$LOG_DIR/fake-greetd.log"
}

# ----------------------------------------------------------- compositor ----

cmd_nested() {
    require_display
    step "Otto in login mode, nested in the current session"
    cargo build --release
    cargo build --release -p otto-greeter
    cargo build -p otto-greeter --example fake_greetd
    start_fake_greetd
    mkdir -p "$LOG_DIR"

    note "Otto launches the greeter itself — verify there is NO dock and NO switcher"
    note "compositor log: $LOG_DIR/otto.log"
    echo

    GREETD_SOCK="$SOCK" \
    OTTO_GREETER_COMMAND="$ROOT/target/release/otto-greeter" \
    RUST_LOG="${RUST_LOG:-info}" \
        ./target/release/otto --winit --login &> "$LOG_DIR/otto.log" || true

    step "Checking the compositor log"
    if grep -q "Login mode: launching greeter" "$LOG_DIR/otto.log"; then
        ok "greeter was launched instead of autostart"
    else
        fail "greeter launch not found in the log"
    fi
}

cmd_tty() {
    step "Otto in login mode on the console"
    echo "${YELLOW}This takes over the display on the current VT.${OFF}"
    echo "Switch VTs (Ctrl+Alt+F<n>) from another terminal to get back if it hangs."
    echo
    read -r -p "Continue? [y/N] " reply
    [[ "$reply" == [yY] ]] || { echo "Aborted."; exit 0; }

    cargo build --release
    cargo build --release -p otto-greeter
    cargo build -p otto-greeter --example fake_greetd
    start_fake_greetd
    mkdir -p "$LOG_DIR"

    note "using a fake greetd — no session will actually start"
    note "compositor log: $LOG_DIR/otto.log"

    GREETD_SOCK="$SOCK" \
    OTTO_GREETER_COMMAND="$ROOT/target/release/otto-greeter" \
    RUST_LOG="${RUST_LOG:-info}" \
        ./target/release/otto --tty-udev --login &> "$LOG_DIR/otto.log" || true

    step "Checking the compositor log"
    grep -c "Login mode: ignoring secondary connector" "$LOG_DIR/otto.log" \
        | xargs -I{} note "secondary connectors ignored: {}"
    if grep -q "Login mode: launching greeter" "$LOG_DIR/otto.log"; then
        ok "greeter was launched"
    else
        fail "greeter launch not found in the log"
    fi
}

# ---------------------------------------------------------- real greetd ----

cmd_greetd() {
    step "Real greetd configuration"
    local config="$LOG_DIR/greetd-config.toml"
    mkdir -p "$LOG_DIR"

    cat > "$config" <<EOF
# Otto as a greetd greeter. Install to /etc/greetd/config.toml.
#
# greetd runs as root, owns the VT, and execs this command as the unprivileged
# 'greeter' user with GREETD_SOCK set. Otto passes that socket to otto-greeter,
# which drives the authentication conversation.

[terminal]
vt = 1

[default_session]
command = "$ROOT/target/release/otto --tty-udev --login"
user = "greeter"
EOF

    cat "$config"
    echo
    note "written to $config"
    echo
    echo "To install and run it:"
    echo "  ${DIM}# otto-greeter must be on PATH, or set login.greeter_command in otto's config${OFF}"
    echo "  pkexec install -Dm644 $config /etc/greetd/config.toml"
    echo "  pkexec systemctl restart greetd"
    echo
    echo "${YELLOW}Test this on a spare VT before enabling greetd at boot${OFF} — a broken"
    echo "greeter on vt1 with greetd enabled can leave you without a way to log in."
    echo "Keep a second VT with a shell open while you try it."
    echo
    if command -v greetd >/dev/null; then
        ok "greetd is installed ($(command -v greetd))"
    else
        warn "greetd is not installed — install it before using this config"
    fi
    if id greeter &>/dev/null; then
        ok "the 'greeter' user exists"
    else
        warn "the 'greeter' user does not exist (greetd's package usually creates it)"
    fi
}

# ------------------------------------------------------------------ all ----

cmd_all() {
    cmd_check
    cmd_ipc
    echo
    step "Summary"
    ok "everything that can run headlessly passed"
    if have_display; then
        note "a Wayland session is available — run '$0 greeter' for the UI"
    else
        note "no Wayland session here; run '$0 tty' on a console for the full test"
    fi
}

usage() {
    # The header comment doubles as the help text: print it up to the first
    # non-comment line, so the two can never drift apart.
    awk 'NR>1 { if ($0 !~ /^#/) exit; sub(/^# ?/, ""); print }' "$0"
    echo
    echo "Usage: $0 [check|ipc|mock|greeter|nested|tty|greetd|all]"
    echo
    echo "Environment:"
    echo "  FAKE_GREETD_SCENARIO   simple | fingerprint | two-factor | locked  (default: fingerprint)"
    echo "  FAKE_GREETD_PASSWORD   password the fake daemon accepts            (default: otto)"
    echo "  LOG_DIR                where logs are written                      (default: /tmp/otto-login-test)"
}

case "${1:-all}" in
    check)   cmd_check ;;
    ipc)     cmd_ipc ;;
    mock)    cmd_mock ;;
    greeter) cmd_greeter ;;
    nested)  cmd_nested ;;
    tty)     cmd_tty ;;
    greetd)  cmd_greetd ;;
    all)     cmd_all ;;
    -h|--help|help) usage ;;
    *)       echo "Unknown command: $1"; echo; usage; exit 1 ;;
esac
