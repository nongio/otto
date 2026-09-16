#!/usr/bin/env bash
# Everything CI runs, in the same order. Run it locally before pushing.
# Set SKIP_SPEC_CHECK=1 to skip the spec drift check, which needs network access.
set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")/.."

step() { printf '\n==> %s\n' "$*"; }

step "rustfmt"
cargo fmt -p otto-agentsd --check

step "clippy"
cargo clippy -p otto-agentsd --all-targets --locked -- -D warnings

step "tests"
cargo test -p otto-agentsd --locked

if [[ "${SKIP_SPEC_CHECK:-0}" != 1 ]]; then
  step "spec drift"
  scripts/sync-spec.sh --check
fi
