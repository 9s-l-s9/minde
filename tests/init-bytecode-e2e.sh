#!/bin/sh
# SPDX-License-Identifier: GPL-3.0-or-later
# Assert that the actual main keyboard policy came from init.go, not Guile's
# evaluator. Covers the absolute-load/package-cache naming regression.
set -eu
cd "$(dirname "$0")/.."
. tests/lib/nested-compositor.sh
OUT=${MINDE_INIT_E2E_OUT:-/tmp/minde-init-bytecode}
trap nested_stop EXIT HUP INT TERM
if [ -z "${MINDE_NESTED_PACKAGE:-}" ]; then
    make compile-scheme
    # Child-only package overrides in nested_start are intentionally isolated.
    # shellcheck disable=SC2031
    export GUILE_LOAD_COMPILED_PATH="$PWD/build/ccache"
    # shellcheck disable=SC2031
    export GUILE_AUTO_COMPILE=0
fi
nested_start "$OUT" "${MINDE_INIT_E2E_DISPLAY:-:94}"
compiled=$(scripts/mindectl eval '(begin
  (use-modules (system vm program))
  (any (lambda (source)
         (and (string? (cadr source))
              (string-suffix? "init.scm" (cadr source))))
       (program-sources wm-handle-key)))')
[ "$compiled" = '#t' ] || {
    echo "error: keyboard policy is interpreted instead of loading init.go" >&2
    exit 1
}
echo "ok - keyboard policy loaded from compiled init.go"
# Custom init files must retain their exact-path semantics even if named
# init.scm; they must not accidentally select the bundled init.go instead.
nested_stop
mkdir -p "$OUT/custom"
cat > "$OUT/custom/init.scm" <<'SCM'
(load-from-path "init")
(define minde-custom-init-marker 'custom-init-loaded)
SCM
export MINDE_NESTED_INIT="$OUT/custom/init.scm"
nested_start "$OUT/custom-session" "${MINDE_INIT_E2E_DISPLAY:-:94}"
marker=$(scripts/mindectl eval 'minde-custom-init-marker')
[ "$marker" = 'custom-init-loaded' ] || exit 1
echo "ok - custom init.scm still loads the explicitly selected file"
