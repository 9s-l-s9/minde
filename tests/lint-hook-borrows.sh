#!/bin/sh
# SPDX-License-Identifier: GPL-3.0-or-later
# Guards the direct command-application contract (guile::set_state): a
# Rust->Scheme call (guile::on_*, handle_key, publish_status, eval_ipc ...)
# may apply wm-* commands to the compositor state before it returns, so the
# caller must not keep a reference into that state alive across the call.
# Flag any function that binds `let NAME = &self.FIELD` / `&mut self.FIELD`
# (or the `state`/`data` receivers the callbacks use), then calls into
# Scheme, then uses NAME again. Ids and clones are fine; references are not.
set -eu
cd "$(dirname "$0")/.."

fail=0
for f in src/*.rs src/**/*.rs; do
  [ -f "$f" ] || continue
  case "$f" in src/guile/*) continue ;; esac
  hits=$(awk '
    function reset() { delete held; nheld = 0; crossed = 0 }
    { line = $0; sub(/\/\/.*/, "", line) }
    line ~ /^[[:space:]]*(pub(\([a-z]+\))?[[:space:]]+)?(unsafe[[:space:]]+)?fn[[:space:]]/ { reset() }
    match(line, /let[[:space:]]+(mut[[:space:]]+)?[A-Za-z_][A-Za-z0-9_]*[[:space:]]*=[[:space:]]*&(mut[[:space:]]+)?(self|state|data)\./) {
      name = substr(line, RSTART, RLENGTH)
      sub(/^let[[:space:]]+(mut[[:space:]]+)?/, "", name)
      sub(/[[:space:]]*=.*/, "", name)
      held[name] = FNR; nheld++
    }
    line ~ /(^|[^A-Za-z0-9_])(crate::)?guile::(on_|handle_key|publish_status|eval_ipc|set_xwayland_status|output_config_allowed)/ {
      if (nheld > 0) crossed = FNR
    }
    crossed && FNR > crossed {
      for (n in held) {
        if (held[n] < crossed && match(line, "(^|[^A-Za-z0-9_])" n "([^A-Za-z0-9_]|$)")) {
          print FILENAME ": " FNR ": `" n "` (bound at line " held[n] ") used after the Scheme call at line " crossed
          delete held[n]
        }
      }
    }
  ' "$f")
  if [ -n "$hits" ]; then
    echo "lint-hook-borrows: a reference into the compositor state is held across a Scheme call:"
    echo "$hits"
    echo "  -> pass ids or clones instead; the call may apply wm-* commands"
    echo "     to that state before it returns (see guile::set_state)."
    fail=1
  fi
done

# Separate guard for the GC-rooting hazard fixed in PLAN.md Epic I, issue I2:
# bdw-gc scans the stack, registers and statics, but never the Rust heap, so
# a `Vec<Scm>` collecting heap-allocated values (pairs, strings, symbols)
# across a further allocating Guile call can be collected out from under a
# later use. This is exactly the src/guile/* code the loop above skips, so
# check it here instead: flag any `Vec<Scm>` in src/guile/mod.rs unless its
# line is marked `gc-safe: fixnums only` (immediate values need no root).
gcfile=src/guile/mod.rs
if [ -f "$gcfile" ]; then
  hits=$(awk '
    /^[[:space:]]*(\/\/\/|\/\/!)/ { next }              # skip doc comments
    /Vec<Scm>/ && !/gc-safe: fixnums only/ { print FILENAME ": " FNR ": " $0 }
  ' "$gcfile")
  if [ -n "$hits" ]; then
    echo "lint-hook-borrows: Vec<Scm> without a gc-safe annotation:"
    echo "$hits"
    echo "  -> a Vec<Scm> lives on the Rust heap, invisible to bdw-gc; build the"
    echo "     list incrementally into a stack-held Scm local instead (see"
    echo "     scm_list_map), or mark the line 'gc-safe: fixnums only' if it"
    echo "     can only ever hold immediate values."
    fail=1
  fi
fi

exit $fail
