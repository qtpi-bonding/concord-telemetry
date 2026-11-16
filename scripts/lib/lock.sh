#!/usr/bin/env bash
# Shared helpers for the scripts in scripts/. Sourcing this file defines
# functions only; it runs nothing.

# The git sources (`git+URL?...#REV`) of the packages named exactly $2 in the
# lock file $1, one per line.
sources_of() {
  awk -v want="name = \"$2\"" '
    /^\[\[package\]\]/ { hit = 0; next }
    $0 == want { hit = 1; next }
    hit && /^source = "git\+[^"]*#[^"]*"/ { s = $0; sub(/^source = "/, "", s); sub(/"$/, "", s); print s }
  ' "$1" | sort -u
}
