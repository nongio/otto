#!/usr/bin/env bash
# Compare the pinned upstream spec with the newest published spec release.
# Exits 1 when a newer spec/vX.Y.Z tag exists.
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
# shellcheck source=../spec/upstream.env
source "$root/spec/upstream.env"

latest="$(git ls-remote --tags --refs "$AHP_REPO" 'spec/v*' | sed 's#.*refs/tags/##' | sort -V | tail -n 1)"

echo "pinned: $AHP_REF (protocol $AHP_PROTOCOL_VERSION, commit $AHP_COMMIT)"
echo "latest: ${latest:-<no spec releases found>}"

if [[ -n "$latest" && "$latest" != "$AHP_REF" ]] &&
  [[ "$(printf '%s\n' "$AHP_REF" "$latest" | sort -V | tail -n 1)" == "$latest" ]]; then
  echo "A newer spec release is available: scripts/sync-spec.sh --ref $latest"
  exit 1
fi
