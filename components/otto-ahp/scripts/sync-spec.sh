#!/usr/bin/env bash
# Vendor the Agent Host Protocol specification from upstream into spec/upstream/.
#
# Usage:
#   scripts/sync-spec.sh                       re-vendor the ref pinned in spec/upstream.env
#   scripts/sync-spec.sh --ref spec/v0.10.0    vendor another tag or branch and move the pin
#   scripts/sync-spec.sh --check               fail if spec/upstream/ differs from the pinned ref
#   scripts/sync-spec.sh --with-reference      also generate the reference docs (needs Node + npm)
#   scripts/sync-spec.sh --without-reference   stop vendoring the reference docs
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
pin="$root/spec/upstream.env"
dest="$root/spec/upstream"

usage() { sed -n '3,9p' "${BASH_SOURCE[0]}" | sed 's/^# \{0,1\}//'; }

# shellcheck source=../spec/upstream.env
source "$pin"
mode=sync
ref="$AHP_REF"
with_reference="${AHP_WITH_REFERENCE:-0}"

while [[ $# -gt 0 ]]; do
  case "$1" in
    --ref) ref="${2:?--ref needs a tag or branch}"; shift ;;
    --check) mode=check ;;
    --with-reference) with_reference=1 ;;
    --without-reference) with_reference=0 ;;
    -h | --help) usage; exit 0 ;;
    *) echo "unknown argument: $1" >&2; usage >&2; exit 2 ;;
  esac
  shift
done

work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
src="$work/src"
out="$work/upstream"

echo "Fetching $AHP_REPO at $ref"
git -c advice.detachedHead=false clone --quiet --depth 1 --branch "$ref" "$AHP_REPO" "$src"
commit="$(git -C "$src" rev-parse HEAD)"
version="$(sed -n 's/^pub const PROTOCOL_VERSION: &str = "\(.*\)";$/\1/p' \
  "$src/clients/rust/crates/ahp-types/src/version.rs")"
if [[ -z "$version" ]]; then
  echo "could not determine the protocol version at $ref" >&2
  exit 1
fi

if [[ "$mode" == check && "$commit" != "$AHP_COMMIT" ]]; then
  echo "$ref now points at $commit, but spec/upstream.env pins $AHP_COMMIT" >&2
  exit 1
fi

mkdir -p "$out"
cp -R "$src/docs/specification" "$out/specification"
cp -R "$src/docs/guide" "$out/guide"
cp -R "$src/schema" "$out/schema"
cp -R "$src/types" "$out/types"
cp "$src/LICENSE" "$out/LICENSE"

if [[ "$with_reference" == 1 ]]; then
  echo "Generating reference docs (npm ci + npm run generate:docs)"
  (cd "$src" && npm ci --ignore-scripts --no-audit --no-fund --loglevel=error && npm run --silent generate:docs)
  cp -R "$src/docs/reference" "$out/reference"
fi

if [[ "$mode" == check ]]; then
  if diff -r "$out" "$dest" >"$work/diff"; then
    echo "spec/upstream matches $ref ($commit)"
    exit 0
  fi
  head -n 50 "$work/diff" >&2
  echo "spec/upstream has drifted from $ref; run scripts/sync-spec.sh" >&2
  exit 1
fi

rm -rf "$dest"
mv "$out" "$dest"
cat >"$pin" <<EOF
# Upstream pin for the vendored spec in spec/upstream/. Managed by scripts/sync-spec.sh.
AHP_REPO=$AHP_REPO
AHP_REF=$ref
AHP_COMMIT=$commit
AHP_PROTOCOL_VERSION=$version
AHP_WITH_REFERENCE=$with_reference
EOF

echo "Vendored $ref ($commit), protocol version $version"
if [[ "$version" != "${AHP_PROTOCOL_VERSION:-}" && -n "${AHP_PROTOCOL_VERSION:-}" ]]; then
  echo "Protocol version changed from $AHP_PROTOCOL_VERSION: bump ahp-types, ahp and ahp-ws in Cargo.toml to =$version"
fi
