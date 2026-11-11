#!/usr/bin/env bash
# Tests scripts/concord-lint-for-lock.sh against fake locks and a stub cargo.
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
script="$here/concord-lint-for-lock.sh"
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
export HOME="$work/home"; mkdir -p "$HOME"

# Stands in for `cargo-slot cargo ...`: logs its arguments and makes the binary
# where --root says.
cat > "$work/stub" <<'S'
#!/usr/bin/env bash
echo "$*" >> "$STUB_LOG"
root=""; while [ $# -gt 0 ]; do [ "$1" = "--root" ] && root="$2"; shift; done
mkdir -p "$root/bin"; : > "$root/bin/concord-lint"; chmod +x "$root/bin/concord-lint"
S
chmod +x "$work/stub"
export CARGO_SLOT="$work/stub" STUB_LOG="$work/log"; : > "$STUB_LOG"

REV=0123456789abcdef0123456789abcdef01234567
lock() { cat > "$work/Cargo.lock" <<L
version = 4

[[package]]
name = "concord-lint-extra"
version = "0.1.0"
source = "git+https://example.com/other.git?rev=ffffffffffffffffffffffffffffffffffffffff#ffffffffffffffffffffffffffffffffffffffff"

[[package]]
name = "concord-lint"
version = "0.1.0"
$1
L
}
fail() { echo "FAIL: $*" >&2; exit 1; }

lock "source = \"git+https://github.com/qtpi-bonding/concord-telemetry.git?rev=$REV#$REV\""
out="$("$script" "$work/Cargo.lock")"
[ "$out" = "$REV" ] || fail "printed '$out', want the concord-lint rev"
[ -x "$HOME/.cache/concord-lint/$REV/bin/concord-lint" ] || fail "linter not installed under the rev"
grep -q -- "cargo install --locked --root $HOME/.cache/concord-lint/$REV --git https://github.com/qtpi-bonding/concord-telemetry.git --rev $REV concord-lint" "$STUB_LOG" \
  || fail "unexpected install command: $(cat "$STUB_LOG")"

# Second run: already installed, no second install, same output.
out="$("$script" "$work/Cargo.lock")"
[ "$out" = "$REV" ] || fail "second run printed '$out'"
[ "$(wc -l < "$STUB_LOG")" -eq 1 ] || fail "reinstalled an installed linter"

# A lock where concord-lint is a path dependency has no rev.
lock ""
if "$script" "$work/Cargo.lock" 2>"$work/err"; then fail "path/no-source lock must fail"; fi
grep -q "no git rev" "$work/err" || fail "missing explanation: $(cat "$work/err")"

# No concord-lint package at all.
printf 'version = 4\n\n[[package]]\nname = "serde"\nversion = "1.0.0"\n' > "$work/Cargo.lock"
if "$script" "$work/Cargo.lock" 2>"$work/err"; then fail "lock without concord-lint must fail"; fi
if "$script" "$work/missing.lock" 2>/dev/null; then fail "missing lock must fail"; fi
echo "concord-lint-for-lock: ok"
