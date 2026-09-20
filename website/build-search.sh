#!/bin/bash
# Build the search index.
#
# Pagefind reads finished HTML rather than the markdown behind it, so the
# site has to exist before it can be indexed. This builds a throwaway copy
# for that, and writes the index into assets/ — from where the real build
# (and `hugo server`) picks it up like any other static file. The index is
# generated, not committed.
set -e

cd "$(dirname "$0")"

SCRATCH="$(mktemp -d)"
trap 'rm -rf "$SCRATCH"' EXIT

# Indexed at the site root: the URLs Pagefind stores are root-relative, and
# search.js prepends the real base at runtime, so one index serves the site
# wherever it is published (including GitHub's /otto/ subpath).
hugo --quiet --destination "$SCRATCH" --baseURL /

npx -y pagefind@1 --site "$SCRATCH" --output-path assets/pagefind > /dev/null
