#!/usr/bin/env bash
# Fails unless the normal dependency tree contains the exact package `opentelemetry`
# (opentelemetry_sdk alone does not count). Checks each -p PACKAGE, or the
# whole workspace when none is given.
#   check-has-telemetry.sh [-p PACKAGE]...
# Cargo runs through $CARGO_SLOT if set, else `cargo-slot` on PATH if present,
# else directly.
set -euo pipefail

usage() { echo "usage: $0 [-p PACKAGE]..." >&2; exit 2; }

pkgs=()
while [ $# -gt 0 ]; do
  case "$1" in
    -p) [ $# -ge 2 ] || usage; pkgs+=(-p "$2"); shift 2 ;;
    *) usage ;;
  esac
done
[ ${#pkgs[@]} -gt 0 ] || pkgs=(--workspace)

SLOT="${CARGO_SLOT-$(command -v cargo-slot || true)}"

tree="$($SLOT cargo tree "${pkgs[@]}" -e normal)" \
  || { echo "cargo tree failed" >&2; exit 1; }
names="$(printf '%s\n' "$tree" | grep -oE '[A-Za-z0-9_-]+ v[0-9]' | awk '{print $1}' | sort -u)"

if ! printf '%s\n' "$names" | grep -qxF opentelemetry; then
  echo "no opentelemetry dependency in: ${pkgs[*]}" >&2
  exit 1
fi
