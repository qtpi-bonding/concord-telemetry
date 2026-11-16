#!/usr/bin/env bash
# Fails if the normal (non-dev, non-build) dependency tree contains telemetry:
# concord-telemetry, concord-telemetry-derive, tracing-opentelemetry, any
# opentelemetry* package, or any extra --also NAME.
#   check-no-telemetry.sh [-p PACKAGE]... [--features F] [--also NAME]...
# -p limits the tree to the named workspace members (default: --workspace).
# Cargo runs through $CARGO_SLOT if set, else `cargo-slot` on PATH if present,
# else directly. The verdict comes from the package names in the tree, not from
# cargo's exit status (`cargo tree -i` of an absent package exits 0).
set -euo pipefail

usage() { echo "usage: $0 [-p PACKAGE]... [--features F] [--also NAME]..." >&2; exit 2; }

pkgs=()
features=()
also=()
while [ $# -gt 0 ]; do
  case "$1" in
    -p) [ $# -ge 2 ] || usage; pkgs+=(-p "$2"); shift 2 ;;
    --features) [ $# -ge 2 ] || usage; features=(--features "$2"); shift 2 ;;
    --also) [ $# -ge 2 ] || usage; also+=("$2"); shift 2 ;;
    *) usage ;;
  esac
done
[ ${#pkgs[@]} -gt 0 ] || pkgs=(--workspace)

SLOT="${CARGO_SLOT-$(command -v cargo-slot || true)}"

tree="$($SLOT cargo tree ${pkgs[@]+"${pkgs[@]}"} ${features[@]+"${features[@]}"} -e normal)" \
  || { echo "cargo tree failed" >&2; exit 1; }
names="$(printf '%s\n' "$tree" | grep -oE '[A-Za-z0-9_-]+ v[0-9]' | awk '{print $1}' | sort -u)"

bad="$(printf '%s\n' "$names" | grep -E '^(concord-telemetry|concord-telemetry-derive|tracing-opentelemetry|opentelemetry.*)$' || true)"
for n in ${also[@]+"${also[@]}"}; do
  if printf '%s\n' "$names" | grep -qxF -- "$n"; then
    bad="$(printf '%s\n%s' "$bad" "$n")"
  fi
done
bad="$(printf '%s\n' "$bad" | sed '/^$/d')"

if [ -n "$bad" ]; then
  echo "telemetry dependencies found: $(printf '%s' "$bad" | tr '\n' ' ')" >&2
  exit 1
fi
