#!/usr/bin/env bash
# Copies the user guides into the otto-help skill, so the agent reading the
# skill has the same pages the documentation site serves and never a
# paraphrase of them that drifted.
#
#   scripts/sync-skill-docs.sh          copy docs/user into the skill
#   scripts/sync-skill-docs.sh --check  fail if a copy is stale (CI)
#
# The skill's own pages under references/ stay hand-written: they carry the
# exact commands and the decision tables a small model needs, which the
# guides deliberately do not. These copies are the long answer behind them.
set -euo pipefail

cd "$(dirname "$0")/.."

SRC="docs/user"
DST="resources/plugins/otto/skills/otto-help/references/docs"

check=0
[ "${1-}" = "--check" ] && check=1

if [ "$check" -eq 1 ]; then
    stale=""
    for f in "$SRC"/*.md; do
        base="$(basename "$f")"
        if ! cmp -s "$f" "$DST/$base"; then
            stale="$stale  $base\n"
        fi
    done
    # A page deleted from docs/user must not linger in the skill.
    for f in "$DST"/*.md; do
        base="$(basename "$f")"
        [ -f "$SRC/$base" ] || stale="$stale  $base (no longer in $SRC)\n"
    done
    if [ -n "$stale" ]; then
        printf 'The skill\x27s copies of the user guides are out of date:\n'
        printf "$stale"
        printf 'Run scripts/sync-skill-docs.sh and commit the result.\n'
        exit 1
    fi
    echo "skill doc copies are current ($(ls "$SRC"/*.md | wc -l) pages)"
    exit 0
fi

mkdir -p "$DST"
rm -f "$DST"/*.md
cp "$SRC"/*.md "$DST/"
chmod 644 "$DST"/*.md
echo "copied $(ls "$DST"/*.md | wc -l) guides into $DST"
