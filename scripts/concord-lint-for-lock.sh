#!/usr/bin/env bash
# Prints the git rev of the concord-lint linter a Cargo.lock is pinned to and
# makes sure that rev's linter is installed:
#   concord-lint-for-lock.sh PATH/TO/Cargo.lock
# The rev comes from the `concord-lint` package's git source in the lock. A repo
# with no concord-lint package (it depends only on concord-telemetry) uses the
# `concord-telemetry` package's git source instead: same repo, same rev. Either
# package locked at more than one rev is an error.
# The linter lives in ~/.cache/concord-lint/<rev>/bin/concord-lint, installed with
# `cargo install --locked --root` when missing. Only the rev goes to stdout.
# Cargo runs through $CARGO_SLOT if set, else `cargo-slot` on PATH if present,
# else directly. The install stays in this script: a cargo-slot call made inside
# another cargo-slot call runs straight through (no second slot is taken), so
# calling this script from a check that already holds a slot cannot deadlock.
set -euo pipefail

[ $# -eq 1 ] || { echo "usage: $0 PATH/TO/Cargo.lock" >&2; exit 2; }
lock="$1"
[ -f "$lock" ] || { echo "no such lock file: $lock" >&2; exit 1; }

# The git sources (`git+URL?...#REV`) of the packages named exactly $1, one per line.
sources_of() {
  awk -v want="name = \"$1\"" '
    /^\[\[package\]\]/ { hit = 0; next }
    $0 == want { hit = 1; next }
    hit && /^source = "git\+[^"]*#[^"]*"/ { s = $0; sub(/^source = "/, "", s); sub(/"$/, "", s); print s }
  ' "$lock" | sort -u
}

package=concord-lint
sources="$(sources_of concord-lint)"
if [ -z "$sources" ]; then
  package=concord-telemetry
  sources="$(sources_of concord-telemetry)"
fi
[ -n "$sources" ] || { echo "no git rev for concord-lint or concord-telemetry in $lock (absent, or path dependencies)" >&2; exit 1; }
revs="$(printf '%s\n' "$sources" | sed 's/.*#//' | sort -u)"
if [ "$(printf '%s\n' "$revs" | wc -l)" -ne 1 ]; then
  echo "$lock has more than one $package rev: $(printf '%s' "$revs" | tr '\n' ' ')" >&2
  exit 1
fi
rev="$revs"
url="$(printf '%s\n' "$sources" | head -n1)"; url="${url#git+}"; url="${url%%#*}"; url="${url%%\?*}"

root="$HOME/.cache/concord-lint/$rev"
if [ ! -x "$root/bin/concord-lint" ]; then
  SLOT="${CARGO_SLOT-$(command -v cargo-slot || true)}"
  # Build in a target dir of its own and install into a temporary root, then
  # move it into place. A shared target dir (a global `target-dir` in
  # ~/.cargo/config.toml) once handed back a stale concord-lint binary from an
  # earlier rev (2026-11-10), and an interrupted install must never leave a
  # bin/concord-lint that later runs trust.
  work="$(mktemp -d)"
  trap 'rm -rf "$work"' EXIT
  $SLOT cargo install --locked --root "$work/root" --target-dir "$work/target" \
    --git "$url" --rev "$rev" concord-lint >&2
  mkdir -p "$(dirname "$root")"
  rm -rf "$root"
  mv "$work/root" "$root"
fi
echo "$rev"
