#!/bin/sh
# SPDX-License-Identifier: GPL-3.0-or-later
# Nested (winit) end-to-end gate for the keyboard layout group surface:
# minde is started with two XKB groups (de bone + us) and the Scheme
# wrappers `keyboard-layouts' / `set-keyboard-layout!' are driven with
# `mindectl eval', checking that the active flag follows the switch and
# that `handle-keyboard-layout-changed!' fires with the group name.
set -eu

cd "$(dirname "$0")/.."
. tests/lib/nested-compositor.sh

OUT=${MINDE_KEYBOARD_LAYOUT_E2E_OUT:-/tmp/swm-kblayout-e2e}
export NESTED_XKB_LAYOUT=de,us NESTED_XKB_VARIANT=bone,
trap nested_stop EXIT HUP INT TERM

nested_start "$OUT" "${MINDE_KEYBOARD_LAYOUT_E2E_DISPLAY:-:99}" || {
    echo "error: nested compositor failed; inspect $OUT" >&2
    exit 1
}

fail() {
    echo "error: $1" >&2
    shift
    for f in "$@"; do cat "$f" >&2 2>/dev/null || true; done
    exit 1
}

seval() {
    timeout 15 scripts/mindectl eval "$1"
}

active() {
    seval '(assq-ref (find (lambda (l) (assq-ref l (quote active))) (keyboard-layouts)) (quote name))'
}

count=$(seval '(length (keyboard-layouts))') || fail "eval (keyboard-layouts) failed"
[ "$count" = "2" ] || fail "expected two layout groups, got $count"
[ "$(active)" = '"German (Bone)"' ] || fail "initial group is not German (Bone): $(active)"
echo "ok - (keyboard-layouts) lists both groups, de(bone) active"

seval '(begin (define %kb-changes (quote ()))
              (define (handle-keyboard-layout-changed! name)
                (set! %kb-changes (cons name %kb-changes)))
              #t)' >/dev/null

r=$(seval '(set-keyboard-layout! (quote next))') || fail "set-keyboard-layout! eval failed"
[ "$r" = "#t" ] || fail "set-keyboard-layout! next did not return #t: $r"
sleep 0.5
[ "$(active)" = '"English (US)"' ] || fail "next did not activate English (US): $(active)"
echo "ok - (set-keyboard-layout! 'next) activates English (US)"

r=$(seval '(set-keyboard-layout! "german")')
sleep 0.5
[ "$(active)" = '"German (Bone)"' ] || fail "name prefix did not activate German (Bone): $(active)"
echo "ok - (set-keyboard-layout! \"german\") switches back by name prefix"

changes=$(seval '%kb-changes')
[ "$changes" = '("German (Bone)" "English (US)")' ] ||
    fail "handle-keyboard-layout-changed! did not see both switches: $changes"
echo "ok - handle-keyboard-layout-changed! fired for each switch"

r=$(seval '(set-keyboard-layout! 7)')
sleep 0.5
[ "$(active)" = '"German (Bone)"' ] || fail "out-of-range index changed the group"
echo "ok - out-of-range index is ignored"

echo "keyboard layout e2e: all checks passed"
