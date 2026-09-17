#!/usr/bin/env python3
"""Which CI areas a Cargo.lock change reaches.

usage: ci-lock-scope.py <base-sha> <head-sha>

A lockfile edit usually moves one crate's dependencies, not the whole
workspace's. For each CI area this walks the dependency closure of the area's
root crates in the base and head lockfiles and prints `<area>=true` when any
package in that closure was added, removed or changed (version, source,
checksum or its own dependency list), `<area>=false` otherwise.

The lockfile records dependencies across every feature and target, so the
closure is a superset of what any one build links: it can say true when a
build would not change, never false when it would.
"""

import subprocess
import sys
import tomllib

# The crates whose closure decides each area. `ui` is every workspace member
# not claimed by the compositor or the two leaf crates, matching the UI path
# filter in ci.yml.
ROOTS = {
    "compositor": {"otto"},
    "rdp": {"otto-rdp"},
    "portal": {"xdg-desktop-portal-otto"},
    "files": {"otto-files", "otto-quickview", "otto-media-kit"},
}
NOT_UI = {"otto", "otto-rdp", "xdg-desktop-portal-otto"}


def load(sha: str) -> list[dict]:
    text = subprocess.run(
        ["git", "show", f"{sha}:Cargo.lock"], capture_output=True, text=True
    )
    if text.returncode != 0:
        return []
    return tomllib.loads(text.stdout).get("package", [])


def closure(packages: list[dict], roots: set[str]) -> set[tuple]:
    by_name: dict[str, list[dict]] = {}
    for pkg in packages:
        by_name.setdefault(pkg["name"], []).append(pkg)

    def resolve(dep: str) -> list[dict]:
        # "name", "name version" or "name version (source)"
        parts = dep.split(" ")
        candidates = by_name.get(parts[0], [])
        if len(parts) > 1:
            candidates = [p for p in candidates if p["version"] == parts[1]]
        return candidates

    seen: dict[tuple, tuple] = {}
    stack = [p for name in roots for p in by_name.get(name, [])]
    while stack:
        pkg = stack.pop()
        key = (pkg["name"], pkg["version"], pkg.get("source"))
        if key in seen:
            continue
        deps = tuple(sorted(pkg.get("dependencies", [])))
        seen[key] = key + (pkg.get("checksum"), deps)
        for dep in deps:
            stack.extend(resolve(dep))
    return set(seen.values())


def main() -> None:
    base, head = load(sys.argv[1]), load(sys.argv[2])
    members = {p["name"] for p in base + head if "source" not in p}
    roots = dict(ROOTS, ui=members - NOT_UI)
    for area, names in roots.items():
        changed = closure(base, names) != closure(head, names)
        print(f"{area}={'true' if changed else 'false'}")


if __name__ == "__main__":
    main()
