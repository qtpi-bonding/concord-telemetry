#!/usr/bin/env bash
# Fails if any crate other than the allowed ones depends directly on `opentelemetry`.
# Consumers run it against their own workspace:
#   check-otel-dependents.sh [-p PACKAGE] ALLOWED_CRATE...
# ALLOWED_CRATE is a crate that may legitimately depend on opentelemetry directly,
# normally just `concord-telemetry` (the OpenTelemetry family below is always
# allowed). `-p PACKAGE` limits the tree to one workspace member; the default is the
# whole workspace in the current directory. Cargo runs through $CARGO_SLOT
# if set, else `cargo-slot` on PATH if present, else directly.
set -euo pipefail

pkg=()
if [ "${1:-}" = "-p" ]; then
  [ $# -ge 2 ] || { echo "usage: $0 [-p PACKAGE] ALLOWED_CRATE..." >&2; exit 2; }
  pkg=(-p "$2"); shift 2
else
  pkg=(--workspace)
fi
[ $# -ge 1 ] || { echo "usage: $0 [-p PACKAGE] ALLOWED_CRATE..." >&2; exit 2; }

SLOT="${CARGO_SLOT-$(command -v cargo-slot || true)}"

# An explicit compare, not a grep.
deps="$($SLOT cargo tree "${pkg[@]}" -e normal -i opentelemetry --depth 1 --prefix none | awk '{print $1}' | sort -u)"
allowed="$(printf '%s\n' "$@" opentelemetry opentelemetry-appender-tracing opentelemetry-http \
  opentelemetry-otlp opentelemetry-proto opentelemetry_sdk tracing-opentelemetry | sort -u)"
unexpected="$(comm -23 <(printf '%s\n' "$deps") <(printf '%s\n' "$allowed"))"
if [ -n "$unexpected" ]; then
  echo "unexpected dependents of opentelemetry: $unexpected" >&2
  exit 1
fi
