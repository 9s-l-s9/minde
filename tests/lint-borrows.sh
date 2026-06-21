#!/bin/sh
# Guards against the RefCell double-borrow class of bug that froze the
# TTY session once (commit 5d960c1): `layer_map_for_output` returns a
# RefCell guard, so calling it twice within one function panics at
# runtime -- and only on the udev backend, which no nested test executes.
# Fail if any function body contains more than one call.
set -eu
cd "$(dirname "$0")/.."

# Dangerous shape: a `let` binding holds the guard for the rest of the
# function, and a later expression calls layer_map_for_output again.
# Scoped temporaries (used and dropped in one expression) are fine.
fail=0
for f in src/*.rs src/**/*.rs; do
  [ -f "$f" ] || continue
  hits=$(awk '
    { line = $0; sub(/\/\/.*/, "", line) }             # ignore comments
    line ~ /^[[:space:]]*(pub[[:space:]]+)?(unsafe[[:space:]]+)?fn[[:space:]]/ { held = 0 }
    line ~ /let[[:space:]].*layer_map_for_output/ { held = 1; next }
    held && line ~ /layer_map_for_output/ { print FILENAME ": " FNR }
  ' "$f")
  if [ -n "$hits" ]; then
    echo "lint-borrows: layer_map_for_output called while a guard binding is live:"
    echo "$hits"
    echo "  -> reuse the existing guard (RefCell double borrow panics at runtime,"
    echo "     and only on the udev backend; see commit 5d960c1)."
    fail=1
  fi
done
exit $fail
