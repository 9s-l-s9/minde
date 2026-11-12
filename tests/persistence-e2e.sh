#!/bin/sh
# SPDX-License-Identifier: GPL-3.0-or-later
# Boot with unreadable saved state; verify deferred diagnostics and recovery.
set -eu
cd "$(dirname "$0")/.."
. tests/lib/nested-compositor.sh

OUT=${MINDE_PERSISTENCE_E2E_OUT:-/tmp/minde-persistence-e2e}
trap nested_stop EXIT HUP INT TERM
mkdir -p "$OUT"
printf '%s\n' '(minde-layouts 2 (("preserved" . leaf)))' >"$OUT/layouts.scm"
printf '%s\n' '(minde-placement-rules 2 (("foot" "II" 0 #f)))' >"$OUT/rules.scm"
cp "$OUT/layouts.scm" "$OUT/layouts.original"
cp "$OUT/rules.scm" "$OUT/rules.original"
nested_start "$OUT" "${MINDE_PERSISTENCE_E2E_DISPLAY:-:93}" || {
    echo "error: nested compositor failed; inspect $OUT" >&2
    exit 1
}

check() {
    expected=$1
    expression=$2
    actual=$(timeout 15 scripts/mindectl eval "$expression")
    if [ "$actual" != "$expected" ]; then
        echo "error: $expression returned $actual, expected $expected" >&2
        exit 1
    fi
}

check '#t' '(and (pair? (wm-outputs))
                 (string? (last-message))
                 (string-contains (last-message) (getenv "MINDE_LAYOUTS_FILE"))
                 (string-contains (last-message) (getenv "MINDE_RULES_FILE")) #t)'
nested_log_has 'could not load minde-layouts' || exit 1
nested_log_has 'could not load minde-placement-rules' || exit 1
echo 'ok - startup failures are logged and shown after the first output'

check '#f' '(save-layout! "blocked")'
check '#f' '(save-placement-rules!)'
cmp "$OUT/layouts.original" "$OUT/layouts.scm"
cmp "$OUT/rules.original" "$OUT/rules.scm"
check '#f' '(layout-spec "blocked")'
echo 'ok - failed saves preserve original bytes and layout registry'

mv "$OUT/layouts.scm" "$OUT/layouts.preserved"
mv "$OUT/rules.scm" "$OUT/rules.preserved"
check '#t' '(save-layout! "recovered")'
check '#t' '(save-placement-rules!)'
check '#t' '(load-layouts!)'
check '#t' '(load-placement-rules!)'
check 'leaf' '(layout-spec "recovered")'
cmp "$OUT/layouts.original" "$OUT/layouts.preserved"
cmp "$OUT/rules.original" "$OUT/rules.preserved"
echo 'ok - moving preserved files aside permits saving and reloading'
echo 'persistence-e2e: all checks passed'
