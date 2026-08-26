#!/usr/bin/env bash
# Set the workspace version — otto and every component crate inherit it via
# [workspace.package], so this one edit versions the whole repo.
#
#   scripts/set-version.sh 1.0.0-rc.2
#
# Pass --tag to also commit and tag the bump (git-cliff reads those tags to
# build CHANGELOG.md, so the tag is what a release actually hangs off).
set -euo pipefail

cd "$(dirname "$0")/.."

version=""
tag=0
for arg in "$@"; do
    case "$arg" in
        --tag) tag=1 ;;
        -*) echo "unknown flag: $arg" >&2; exit 2 ;;
        *) version="$arg" ;;
    esac
done

if [[ -z "$version" ]]; then
    echo "current: $(sed -n '/^\[workspace.package\]/,/^\[/p' Cargo.toml | grep -m1 '^version = ' | cut -d'"' -f2)"
    echo "usage: scripts/set-version.sh <version> [--tag]" >&2
    exit 2
fi

# Semver, with an optional pre-release suffix (1.0.0-rc.2).
if [[ ! "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.-]+)?$ ]]; then
    echo "not a semver version: $version" >&2
    exit 2
fi

# Rewrite the version inside [workspace.package] only — the [dependencies.*]
# tables above it carry `version = ` lines of their own.
python3 - "$version" <<'PY'
import pathlib, re, sys
version = sys.argv[1]
p = pathlib.Path("Cargo.toml")
text = p.read_text()
new, n = re.subn(
    r'(?ms)^(\[workspace\.package\]\n(?:(?!^\[).*?\n)??)version = "[^"]+"$',
    lambda m: m.group(1) + f'version = "{version}"',
    text, count=1)
if n != 1:
    sys.exit("no [workspace.package] version found in Cargo.toml")
p.write_text(new)
PY

# The binary PKGBUILD names the release it downloads, and nothing else derives
# it from Cargo.toml — leave it behind and `curl PKGBUILD && makepkg` keeps
# installing the previous release, with no error to show for it.
#
#   _ver   Cargo's version, which names the release tarball and its top directory
#   _tag   the git tag, v$version, exactly what --tag creates below
#   pkgver the same with hyphens dropped, since pkgver may not contain any
#          (1.0.0-rc.2 -> 1.0.0rc.2, which vercmp sorts above 1.0.0rc1 and
#          below the eventual 1.0.0)
python3 - "$version" <<'PKG'
import pathlib, re, sys
version = sys.argv[1]
path = pathlib.Path("PKGBUILD")
text = path.read_text()
for key, value in (
    ("pkgver", version.replace("-", "")),
    ("_ver", version),
    ("_tag", "v" + version),
):
    text, n = re.subn(r"(?m)^%s=.*$" % key, "%s=%s" % (key, value), text, count=1)
    if n != 1:
        sys.exit("no %s= assignment found in PKGBUILD" % key)
path.write_text(text)
PKG

echo "PKGBUILD pointed at v$version"

# Refresh Cargo.lock (untracked here, but keep the local one consistent).
cargo metadata --format-version 1 >/dev/null

echo "workspace version set to $version"

if [[ "$tag" == 1 ]]; then
    git add Cargo.toml PKGBUILD
    git commit -m "chore(release): $version"
    git tag "v$version"
    echo "committed and tagged v$version — push with: git push --follow-tags"
fi
