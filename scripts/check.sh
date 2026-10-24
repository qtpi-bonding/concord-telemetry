#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
SLOT="${CARGO_SLOT:-$(command -v cargo-slot || true)}"   # compile slots where available; plain cargo elsewhere
$SLOT cargo fmt --all -- --check
$SLOT cargo clippy --workspace --all-targets --all-features -- -D warnings
$SLOT cargo test --workspace --all-features
$SLOT cargo check -p concord-telemetry --no-default-features
# Consumers must never see opentelemetry directly (explicit depth-1 allowlist; shared with consumers).
CARGO_SLOT="$SLOT" scripts/check-otel-dependents.sh -p concord-telemetry concord-telemetry
# No tonic in this crate's tree. `cargo tree -i tonic` exits non-zero when the package is absent (the pass case),
# and an `if` (not a `!` pipeline) is what lets `set -e` see a failure.
if $SLOT cargo tree -p concord-telemetry -e normal -i tonic >/dev/null 2>&1; then
  echo "tonic is in concord-telemetry's tree" >&2; exit 1
fi
# No TLS stack in the default build (plain HTTP; TLS only behind the `tls` feature).
if $SLOT cargo tree -p concord-telemetry -e normal -i rustls >/dev/null 2>&1; then
  echo "rustls is in concord-telemetry's default tree; it belongs behind the tls feature" >&2; exit 1
fi
# The lint passes on this repo itself.
$SLOT cargo run -q -p concord-lint -- concord-telemetry/src concord-lint/src
