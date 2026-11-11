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
# No concord-lint package: fall back to the concord-telemetry package (same repo and rev).
tele() { cat > "$work/Cargo.lock" <<L
version = 4

[[package]]
name = "concord-telemetry-derive"
version = "0.1.0"
source = "git+https://example.com/other.git#ffffffffffffffffffffffffffffffffffffffff"

[[package]]
name = "concord-telemetry"
version = "0.4.0"
$1
L
}
REV2=89abcdef0123456789abcdef0123456789abcdef
rm -rf "$HOME/.cache/concord-lint"; : > "$STUB_LOG"
tele "source = \"git+https://github.com/qtpi-bonding/concord-telemetry.git?rev=$REV2#$REV2\""
out="$("$script" "$work/Cargo.lock")"
[ "$out" = "$REV2" ] || fail "fallback printed '$out'"
grep -q -- "--git https://github.com/qtpi-bonding/concord-telemetry.git --rev $REV2 concord-lint" "$STUB_LOG" \
  || fail "fallback install: $(cat "$STUB_LOG")"

# concord-lint wins over concord-telemetry when both are there.
cat > "$work/Cargo.lock" <<L
version = 4

[[package]]
name = "concord-telemetry"
version = "0.4.0"
source = "git+https://github.com/qtpi-bonding/concord-telemetry.git#$REV2"

[[package]]
name = "concord-lint"
version = "0.1.0"
source = "git+https://github.com/qtpi-bonding/concord-telemetry.git#$REV"
L
[ "$("$script" "$work/Cargo.lock")" = "$REV" ] || fail "concord-lint should win"

# Two revs of the same package: a clear failure, nothing installed.
rm -rf "$HOME/.cache/concord-lint"; : > "$STUB_LOG"
for pkg in concord-lint concord-telemetry; do
  cat > "$work/Cargo.lock" <<L
version = 4

[[package]]
name = "$pkg"
version = "0.1.0"
source = "git+https://github.com/qtpi-bonding/concord-telemetry.git#$REV"

[[package]]
name = "$pkg"
version = "0.2.0"
source = "git+https://github.com/qtpi-bonding/concord-telemetry.git#$REV2"
L
  if "$script" "$work/Cargo.lock" 2>"$work/err"; then fail "two $pkg revs must fail"; fi
  grep -q "more than one" "$work/err" || fail "two $pkg revs: $(cat "$work/err")"
  [ ! -s "$STUB_LOG" ] || fail "installed despite two revs"
done

# Two entries at the same rev are fine.
cat > "$work/Cargo.lock" <<L
version = 4

[[package]]
name = "concord-lint"
version = "0.1.0"
source = "git+https://github.com/qtpi-bonding/concord-telemetry.git#$REV"

[[package]]
name = "concord-lint"
version = "0.1.0"
source = "git+https://github.com/qtpi-bonding/concord-telemetry.git?rev=$REV#$REV"
L
[ "$("$script" "$work/Cargo.lock")" = "$REV" ] || fail "same rev twice should pass"
echo "concord-lint-for-lock: ok"
