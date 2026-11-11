#!/usr/bin/env bash
# Prints the git rev of the `concord-lint` package in a Cargo.lock and makes
# sure that rev's linter is installed:
#   concord-lint-for-lock.sh PATH/TO/Cargo.lock
# The linter lives in ~/.cache/concord-lint/<rev>/bin/concord-lint, installed
# with `cargo install --locked --root` when missing, so each consumer lints with
# the version it is locked to. Only the rev goes to stdout. Cargo runs through
# $CARGO_SLOT if set, else `cargo-slot` on PATH if present, else directly.
set -euo pipefail

[ $# -eq 1 ] || { echo "usage: $0 PATH/TO/Cargo.lock" >&2; exit 2; }
lock="$1"
[ -f "$lock" ] || { echo "no such lock file: $lock" >&2; exit 1; }

# The source of the package named exactly `concord-lint`: `git+URL?...#REV`.
source_line="$(awk '
  /^\[\[package\]\]/ { want = 0; next }
  $0 == "name = \"concord-lint\"" { want = 1; next }
  want && /^source = / { print; exit }
' "$lock")"
[ -n "$source_line" ] || { echo "no git rev for concord-lint in $lock (absent, or a path dependency)" >&2; exit 1; }
source="${source_line#source = \"}"; source="${source%\"}"
case "$source" in git+*'#'*) ;; *) echo "no git rev for concord-lint in $lock (source: $source)" >&2; exit 1 ;; esac
rev="${source##*#}"
url="${source#git+}"; url="${url%%#*}"; url="${url%%\?*}"

root="$HOME/.cache/concord-lint/$rev"
if [ ! -x "$root/bin/concord-lint" ]; then
  SLOT="${CARGO_SLOT-$(command -v cargo-slot || true)}"
  $SLOT cargo install --locked --root "$root" --git "$url" --rev "$rev" concord-lint >&2
fi
echo "$rev"
