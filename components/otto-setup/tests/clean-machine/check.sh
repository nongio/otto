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
#
# A scenario fails on any missing line, printing the output it looked in.
set -uo pipefail

scenario="${1:?usage: check.sh fresh|old-node|apply}"
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
    expect "cc-connect missing"   "cc-connect      not installed" <<<"$out"

    echo "== fresh: dictation --check =="
    rc 1 otto-setup dictation --check
    out=$(otto-setup dictation --check 2>&1)
    # No systemd in the image: the files alone say it is not installed.
    expect "no engine"          "Parakeet        not installed: missing its server and model" <<<"$out"
    expect "no server"          "Speech server   nothing at 127.0.0.1:8080" <<<"$out"
    expect "no otto-dictate"    "Other fields    otto-dictate not installed" <<<"$out"
    expect "no engine blocks"   "[!!  ] Speech server      no engine is installed yet" <<<"$out"

    echo "== fresh: screens =="
    expect "welcome names the topics" "Otto Agents" < <(screen)
    expect "welcome lists dictation"  "Dictation" < <(screen)
    page=$(screen down enter)
    expect "engines not installed" "Parakeet          not installed, Enter for how" <<<"$page"
    page=$(screen down enter enter)
    expect "apt build tools"       '$ sudo apt install build-essential cmake git' <<<"$page"
    expect "clone the source"      '$ git clone https://github.com/nongio/otto ~/otto' <<<"$page"
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

*)
    echo "unknown scenario: $scenario" >&2; exit 2 ;;
esac

if (( fail )); then
    echo "FAILED: $scenario"
    exit 1
fi
echo "PASS: $scenario"
