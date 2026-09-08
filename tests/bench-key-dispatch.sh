#!/bin/sh
# SPDX-License-Identifier: GPL-3.0-or-later
# Compare the actual initialized keyboard policy with compiled and interpreted
# init.scm, inside an isolated compositor. Never targets the desktop session.
set -eu
cd "$(dirname "$0")/.."
. tests/lib/nested-compositor.sh
OUT=${MINDE_KEY_BENCH_OUT:-/tmp/minde-bench-key-dispatch}
trap nested_stop EXIT HUP INT TERM
if [ -z "${MINDE_NESTED_PACKAGE:-}" ]; then
    make compile-scheme
    # Overrides inside nested_start's child do not affect this environment.
    # shellcheck disable=SC2031
    export GUILE_LOAD_COMPILED_PATH="$PWD/build/ccache"
    # shellcheck disable=SC2031
    export GUILE_AUTO_COMPILE=0
fi
nested_start "$OUT" "${MINDE_KEY_BENCH_DISPLAY:-:92}"
: >"$OUT/measurements.txt"
for mode in compiled interpreted compiled; do
    case "$mode" in
        compiled) loader='(load-from-path "init")'; expected='#t' ;;
        interpreted) loader='(load (string-append (getenv "MINDE_SCHEME_DIR") "/init.scm"))'; expected='#f' ;;
    esac
    # Loop and timer run inside Guile: process startup and IPC transport are
    # excluded. The simple unbound key is forwarded, with no window mutation.
    expression="(begin (use-modules (system vm program)) $loader
      (unless (eq? $expected
                    (any (lambda (source)
                           (and (string? (cadr source))
                                (string-suffix? \"init.scm\" (cadr source))))
                         (program-sources wm-handle-key)))
        (error \"benchmark did not load the requested execution mode\"))
      (let warm ((n 100))
        (when (> n 0) (wm-handle-key 0 97 \"a\" \"a\") (warm (- n 1))))
      (let ((start (get-internal-real-time)))
        (let loop ((n 1000))
          (when (> n 0) (wm-handle-key 0 97 \"a\" \"a\") (loop (- n 1))))
        (* 1000.0 (/ (- (get-internal-real-time) start)
                     internal-time-units-per-second))))"
    # Total milliseconds over 1000 calls is numerically microseconds per call.
    elapsed=$(scripts/mindectl eval "$expression")
    printf '%s key policy: %s us/call\n' "$mode" "$elapsed" >>"$OUT/measurements.txt"
done
cat "$OUT/measurements.txt"
