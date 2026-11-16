#!/usr/bin/env bash
# Tests lib/lock.sh, concord-lint-for-lock.sh (--path), check-no-telemetry.sh and
# check-has-telemetry.sh against a stub cargo (CARGO_SLOT) that prints real
# `cargo tree` output. Like real cargo, the stub exits 0 and prints only a
# warning for `cargo tree -i <absent package>`, so scripts cannot decide by exit status.
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
export HOME="$work/home"; mkdir -p "$HOME"
fail() { echo "FAIL: $*" >&2; exit 1; }

cat > "$work/stub" <<'S'
#!/usr/bin/env bash
echo "$*" >> "$STUB_LOG"
case "$*" in
  *"cargo tree"*)
    for a in "$@"; do
      if [ "$a" = "-i" ] || [ "$a" = "--invert" ]; then
        echo "warning: nothing to print." >&2; exit 0
      fi
    done
    cat "$STUB_TREE" ;;
  *"cargo install"*)
    root=""; while [ $# -gt 0 ]; do [ "$1" = "--root" ] && root="$2"; shift; done
    mkdir -p "$root/bin"; : > "$root/bin/concord-lint"; chmod +x "$root/bin/concord-lint" ;;
esac
S
chmod +x "$work/stub"
export CARGO_SLOT="$work/stub" STUB_LOG="$work/log" STUB_TREE="$work/tree"
: > "$STUB_LOG"

# Real `cargo tree -e normal` format (default prefix).
clean_tree() { cat > "$STUB_TREE" <<'T'
app v0.1.0 (/work/app)
├── anyhow v1.0.86
├── serde v1.0.203
│   └── serde_derive v1.0.203 (proc-macro)
│       ├── proc-macro2 v1.0.86
│       └── syn v2.0.68
└── tracing v0.1.40
    └── tracing-core v0.1.32
T
}
tree_with() { clean_tree; printf '└── %s v0.1.0\n' "$@" >> "$STUB_TREE"; }

REV=0123456789abcdef0123456789abcdef01234567
write_lock() { cat > "$work/Cargo.lock" <<L
version = 4

[[package]]
name = "concord-lint"
version = "0.1.0"
source = "git+https://github.com/example/concord-telemetry.git?rev=$REV#$REV"
L
}

path_flag_prints_binary_path() {
  write_lock
  out="$("$here/concord-lint-for-lock.sh" --path "$work/Cargo.lock")"
  [ "$out" = "$HOME/.cache/concord-lint/$REV/bin/concord-lint" ] || fail "--path printed '$out'"
  [ -x "$out" ] || fail "--path binary not installed"
}

no_flag_prints_rev() {
  write_lock
  out="$("$here/concord-lint-for-lock.sh" "$work/Cargo.lock")"
  [ "$out" = "$REV" ] || fail "no flag printed '$out'"
}

lock_sh_sourced_runs_nothing() {
  out="$(bash -c 'set -euo pipefail; source "$1/lib/lock.sh"; declare -F sources_of >/dev/null' _ "$here" 2>&1)" \
    || fail "sourcing lib/lock.sh failed or sources_of undefined: $out"
  [ -z "$out" ] || fail "sourcing printed: $out"
  write_lock
  out="$(bash -c 'source "$1/lib/lock.sh"; sources_of "$2" concord-lint' _ "$here" "$work/Cargo.lock")"
  [ "$out" = "git+https://github.com/example/concord-telemetry.git?rev=$REV#$REV" ] || fail "sources_of LOCK NAME gave '$out'"
}

no_telemetry_passes_on_clean_tree() {
  clean_tree
  "$here/check-no-telemetry.sh" || fail "clean tree rejected"
  "$here/check-no-telemetry.sh" -p app --features x || fail "clean tree with -p/--features rejected"
  grep -q "cargo tree.* -p app" "$STUB_LOG" || fail "-p not passed"
  grep -q -- "-e normal" "$STUB_LOG" || fail "-e normal not used"
}

no_telemetry_fails_on_opentelemetry_sdk() {
  tree_with opentelemetry_sdk
  if "$here/check-no-telemetry.sh" 2>"$work/err"; then fail "opentelemetry_sdk accepted"; fi
  grep -q opentelemetry_sdk "$work/err" || fail "offender not printed: $(cat "$work/err")"
  for n in concord-telemetry concord-telemetry-derive tracing-opentelemetry opentelemetry; do
    tree_with "$n"
    if "$here/check-no-telemetry.sh" 2>/dev/null; then fail "$n accepted"; fi
  done
}

no_telemetry_also_catches_extra_name() {
  tree_with tonic
  "$here/check-no-telemetry.sh" || fail "tonic rejected without --also"
  if "$here/check-no-telemetry.sh" --also tonic 2>"$work/err"; then fail "--also tonic accepted"; fi
  grep -q tonic "$work/err" || fail "offender not printed"
  # a prefix of another package name is not a match
  tree_with tonic-build
  "$here/check-no-telemetry.sh" --also tonic || fail "--also tonic matched tonic-build"
}

has_telemetry_passes_and_fails() {
  tree_with opentelemetry
  "$here/check-has-telemetry.sh" -p app || fail "tree with opentelemetry rejected"
  clean_tree
  if "$here/check-has-telemetry.sh" -p app 2>/dev/null; then fail "clean tree accepted by has-telemetry"; fi
  tree_with opentelemetry_sdk
  if "$here/check-has-telemetry.sh" -p app 2>/dev/null; then fail "opentelemetry_sdk alone satisfies has-telemetry"; fi
}

for t in path_flag_prints_binary_path no_flag_prints_rev lock_sh_sourced_runs_nothing \
  no_telemetry_passes_on_clean_tree no_telemetry_fails_on_opentelemetry_sdk \
  no_telemetry_also_catches_extra_name has_telemetry_passes_and_fails; do
  : > "$STUB_LOG"; rm -rf "$HOME/.cache"
  "$t"; echo "ok $t"
done
echo "check-scripts: ok"
