#!/usr/bin/env bash
# The full gate. Every cargo call goes through the compile slot ($CARGO_SLOT, else
# cargo-slot on PATH, else plain cargo). Negative dependency gates are decided by the
# package names in `cargo tree -e normal`, never by `cargo tree -i`'s exit status
# (with current cargo it exits 0 and prints "nothing to print" for an absent package).
set -euo pipefail
cd "$(dirname "$0")/.."
SLOT="${CARGO_SLOT:-$(command -v cargo-slot || true)}"
export CARGO_SLOT="$SLOT"
step() { echo "== $*" >&2; }

# tree_names ARGS...: the package names in `cargo tree ARGS -e normal`, one per line.
tree_names() {
  local tree
  tree="$($SLOT cargo tree "$@" -e normal)" || { echo "cargo tree failed: $*" >&2; exit 1; }
  printf '%s\n' "$tree" | grep -oE '[A-Za-z0-9_-]+ v[0-9]' | awk '{print $1}' | sort -u
}
# absent PKG ARGS...: fails when PKG is in the tree.
absent() {
  local pkg="$1"; shift
  if tree_names "$@" | grep -qxF "$pkg"; then
    echo "$pkg is in the tree of: cargo tree $*" >&2; exit 1
  fi
}
# present PKG ARGS...: fails when PKG is not in the tree.
present() {
  local pkg="$1"; shift
  if ! tree_names "$@" | grep -qxF "$pkg"; then
    echo "$pkg is missing from the tree of: cargo tree $*" >&2; exit 1
  fi
}

step "1 fmt"
$SLOT cargo fmt --all -- --check

step "2 clippy, all features"
$SLOT cargo clippy --workspace --all-targets --all-features -- -D warnings

step "3 clippy, no default features"
$SLOT cargo clippy -p concord-telemetry --all-targets --no-default-features -- -D warnings

step "4 tests"
$SLOT cargo test --workspace --all-features
$SLOT cargo test -p concord-telemetry --no-default-features
$SLOT cargo test -p concord-telemetry --no-default-features --features grpc,channel,http-server
$SLOT cargo test -p concord-telemetry --features lifecycle,grpc-status

step "5 off build"
$SLOT cargo check -p concord-telemetry --no-default-features

step "6 telemetry in / out of the tree"
# Off build: no opentelemetry (decided by package name, see the header).
absent opentelemetry -p concord-telemetry --no-default-features
absent tracing-opentelemetry -p concord-telemetry --no-default-features
# Default features: opentelemetry present (positive gate); `testing` alone also brings it in.
scripts/check-has-telemetry.sh -p concord-telemetry
present opentelemetry -p concord-telemetry --no-default-features --features testing

step "7 opentelemetry dependents"
# Consumers must never see opentelemetry directly (explicit depth-1 allowlist; shared with consumers).
scripts/check-otel-dependents.sh -p concord-telemetry concord-telemetry

step "8 tonic and rustls"
absent tonic -p concord-telemetry --features grpc
present tonic -p concord-telemetry --features grpc-status
absent rustls -p concord-telemetry

step "9 script tests"
scripts/test-concord-lint-for-lock.sh
scripts/test-check-scripts.sh

step "10 self-lint against the baseline"
# The baseline may only shrink: a new violation fails the run, and so does a
# regenerated baseline that is larger than the committed one.
lint_paths=(concord-telemetry/src concord-lint/src concord-fields/src)
$SLOT cargo run -q -p concord-lint -- --baseline lint-baseline.txt "${lint_paths[@]}"
fresh="$(mktemp)"; trap 'rm -f "$fresh"' EXIT
$SLOT cargo run -q -p concord-lint -- --baseline "$fresh" --write-baseline "${lint_paths[@]}"
count() { awk -F'\t' 'NF >= 2 { s += $1 } END { print s + 0 }' "$1"; }
if [ "$(count "$fresh")" -gt "$(count lint-baseline.txt)" ]; then
  echo "lint baseline would grow ($(count lint-baseline.txt) -> $(count "$fresh"))" >&2; exit 1
fi
echo "check.sh: all steps passed" >&2
