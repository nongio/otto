#!/usr/bin/env bash
# Every check this crate has to pass, in the order CI applies them. A superset of
# CI's own steps, which are workspace-wide and skip the spec drift check.
# Set SKIP_SPEC_CHECK=1 to skip the spec drift check, which needs network access.
set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")/.."

step() { printf '\n==> %s\n' "$*"; }

step "rustfmt"
cargo fmt -p otto-agents --check

step "clippy"
cargo clippy -p otto-agents --all-targets --locked -- -D warnings

step "tests"
cargo test -p otto-agents --locked

if [[ "${SKIP_SPEC_CHECK:-0}" != 1 ]]; then
  step "spec drift"
  scripts/sync-spec.sh --check
fi
