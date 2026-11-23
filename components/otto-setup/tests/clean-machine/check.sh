#!/usr/bin/env bash
# What otto-setup should say on a machine that has nothing yet. Runs inside
# the container the Dockerfile beside this builds, as an ordinary user, one
# scenario at a time:
#
#   check.sh fresh      nothing installed: everything is missing, apt is the
#                       package manager, the instructions say so
#   check.sh old-node   after `apt install nodejs npm`: the Node.js the
#                       distribution ships is too old for the adapters
#   check.sh apply      with a stand-in `claude` on PATH: Claude Code is ready,
#                       --apply writes agents.toml, --check then knows it
#   check.sh herdr      in a home of its own, stand-ins for herdr (`hc`,
#                       `herdr-app`) and `claude`: sessions run in herdr,
#                       Codex is not offered, --apply writes the plain program
#
# A scenario fails on any missing line, printing the output it looked in.
set -uo pipefail

scenario="${1:?usage: check.sh fresh|old-node|apply|herdr}"
here="$(dirname "$0")"
fail=0

# expect <label> <text>: the text on standard input has to contain <text>.
expect() {
    local label="$1" want="$2" got
    got=$(cat)
    if grep -qF -- "$want" <<<"$got"; then
        echo "  ok  $label"
    else
        echo "MISSING ($label): $want"
        sed 's/^/      │ /' <<<"$got"
        fail=1
    fi
}

# screen <keys...>: the screen otto-setup shows after those keys.
screen() {
    python3 -I "$here/screen.py" /usr/bin/otto-setup "$@"
}

# rc <expected> <command...>: the command's exit code has to be <expected>.
rc() {
    local want="$1"; shift
    "$@" >/dev/null 2>&1
    local got=$?
    if [[ "$got" == "$want" ]]; then
        echo "  ok  $* exits $want"
    else
        echo "WRONG EXIT: $* exited $got, expected $want"
        fail=1
    fi
}

case "$scenario" in
fresh)
    echo "== fresh: --check =="
    rc 0 otto-setup --help
    rc 0 otto-setup --version
    rc 1 otto-setup --check
    # Not a terminal: no option behaves as --check.
    rc 1 otto-setup
    out=$(otto-setup --check 2>&1)
    expect "Node.js missing"      "Node.js         not installed" <<<"$out"
    expect "Claude Code missing"  "Claude Code     needs claude" <<<"$out"
    expect "Codex missing"        "Codex           needs codex" <<<"$out"
    expect "OpenCode missing"     "OpenCode        needs opencode" <<<"$out"
    expect "Hermes missing"       "Hermes          needs hermes" <<<"$out"
    expect "pi missing"           "pi              needs pi" <<<"$out"

    echo "== fresh: screens =="
    expect "welcome names the topic" "Otto Agents" < <(screen)
    page=$(screen enter tab)
    expect "Node.js row"            "Node.js           not installed · Enter for how" <<<"$page"
    expect "header counts the fix"  "to fix" <<<"$page"
    page=$(screen enter tab enter)
    expect "apt command"            '$ sudo apt install nodejs npm' <<<"$page"
    expect "nvm alternative"        'nvm install 22' <<<"$page"
    ;;

old-node)
    echo "== old Node.js from apt: $(node --version) =="
    rc 1 otto-setup --check
    out=$(otto-setup --check 2>&1)
    expect "Node.js too old" "is too old: needs 22 or newer" <<<"$out"
    page=$(screen enter tab)
    expect "Node.js row" "is too old: needs 22 or newer · Enter for how" <<<"$page"
    page=$(screen enter tab enter)
    expect "why"         "the Claude Code adapter needs 22" <<<"$page"
    expect "Debian note" "Debian and Ubuntu's own nodejs package is often older than 22" <<<"$page"
    ;;

apply)
    echo "== a stand-in claude: --apply =="
    mkdir -p "$HOME/.local/bin"
    printf '#!/bin/sh\necho "claude stand-in"\n' > "$HOME/.local/bin/claude"
    chmod +x "$HOME/.local/bin/claude"
    out=$(otto-setup --check 2>&1)
    expect "Claude Code ready" "Claude Code     ready to add" <<<"$out"
    expect "adapter from npx"  "Claude Code downloads its own" <<<"$out"

    out=$(otto-setup --apply 2>&1); status=$?
    echo "  (--apply exited $status; there is no systemd here, so the service steps cannot pass)"
    sed 's/^/      │ /' <<<"$out"
    expect "wrote the agents"   "[ok  ] Agents" <<<"$out"
    expect "linked the skills"  "[ok  ] Teach the agents" <<<"$out"
    if [[ -L "$HOME/.agents/skills/otto-help" ]]; then
        echo "  ok  ~/.agents/skills/otto-help is a link"
    else
        echo "MISSING: ~/.agents/skills/otto-help"; fail=1
    fi
    config="$HOME/.config/otto/agents.toml"
    if [[ -f "$config" ]]; then
        echo "  ok  $config written"
        expect "Claude Code agent" "claude-agent-acp" < "$config"
    else
        echo "MISSING: $config"; fail=1
    fi
    out=$(otto-setup --check 2>&1)
    expect "Claude Code added" "Claude Code     already added" <<<"$out"
    ;;

herdr)
    echo "== herdr installed: stand-ins for hc, herdr-app and claude =="
    export HOME="$(mktemp -d)"
    export PATH="$HOME/.local/bin:$PATH"
    mkdir -p "$HOME/.local/bin"
    for program in hc herdr-app claude; do
        printf '#!/bin/sh\necho "%s stand-in"\n' "$program" > "$HOME/.local/bin/$program"
        chmod +x "$HOME/.local/bin/$program"
    done
    out=$(otto-setup --check 2>&1)
    expect "herdr runs the sessions" "Sessions run in herdr (herdr is installed)" <<<"$out"
    expect "Claude Code ready"       "Claude Code     ready to add" <<<"$out"
    expect "Codex not offered"       "Codex           runs only in Otto, not in herdr" <<<"$out"
    expect "pi needs only pi"        "pi              needs pi" <<<"$out"
    expect "herdr found"             "herdr           hc ~/.local/bin/hc" <<<"$out"
    page=$(screen enter tab)
    expect "herdr row"               "herdr             hc, herdr-app found" <<<"$page"
    page=$(screen enter tab down enter)
    expect "sign-in command"         '$ claude auth login' <<<"$page"

    out=$(otto-setup --apply 2>&1); status=$?
    echo "  (--apply exited $status; there is no systemd here, so the service steps cannot pass)"
    sed 's/^/      │ /' <<<"$out"
    expect "wrote the agents" "[ok  ] Agents" <<<"$out"
    config="$HOME/.config/otto/agents.toml"
    expect "the plain program" 'command = "claude"' < "$config"
    if grep -q -e claude-agent-acp -e '^backend' -e '^enter' "$config"; then
        echo "WRONG: $config has an adapter, a backend or an enter line"; fail=1
    else
        echo "  ok  no adapter, no backend key, no enter lines"
    fi
    out=$(otto-setup --check 2>&1)
    expect "Claude Code added" "Claude Code     already added" <<<"$out"
    ;;

*)
    echo "unknown scenario: $scenario" >&2; exit 2 ;;
esac

if (( fail )); then
    echo "FAILED: $scenario"
    exit 1
fi
echo "PASS: $scenario"
