#!/bin/sh
# SPDX-License-Identifier: GPL-3.0-or-later
# Bounded nested e2e gate for the Scheme output API: `output-heads' and
# `configure-output!' from (minde groups), evaluated through the IPC socket
# with `mindectl eval', and cross-checked with wlr-randr (the canonical
# wlr-output-management client). Covers: the head snapshot lists the winit
# head with its identity, mode list and current state; a scale change made
# from Scheme is reflected back to wlr-randr and to `output-heads'; a
# rejected change (mode change under winit, unknown head) reaches
# `handle-output-configure-failed!' and leaves the head untouched; a
# malformed setting is a Scheme error rather than a silent no-op. Every
# client call is wrapped in `timeout`.
set -eu
cd "$(dirname "$0")/.."
. tests/lib/nested-compositor.sh

OUT=${MINDE_OUTPUT_SCHEME_E2E_OUT:-/tmp/swm-outscheme-e2e}
unset MINDE_OUTPUT_MGMT_ALLOW_NO_HEADS
trap nested_stop EXIT HUP INT TERM

command -v wlr-randr >/dev/null 2>&1 || {
    echo "error: wlr-randr is required (wlr-output-management client)" >&2
    exit 127
}

nested_start "$OUT" "${MINDE_OUTPUT_SCHEME_E2E_DISPLAY:-:99}" || {
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

# --- output-heads lists the winit head ------------------------------------
heads=$(seval '(output-heads)') || fail "eval (output-heads) failed"
printf '%s\n' "$heads" >"$OUT/heads.txt"
printf '%s' "$heads" | grep -q '(name . "winit")' ||
    fail "(output-heads) did not list the winit head" "$OUT/heads.txt"
printf '%s' "$heads" | grep -q '(enabled . #t)' ||
    fail "(output-heads) did not report the head enabled" "$OUT/heads.txt"
printf '%s' "$heads" | grep -q '(modes (' ||
    fail "(output-heads) reported no mode list" "$OUT/heads.txt"
printf '%s' "$heads" | grep -q '(transform . ' ||
    fail "(output-heads) did not report the transform" "$OUT/heads.txt"
count=$(seval '(length (output-heads))')
[ "$count" = "1" ] || fail "expected exactly one head, got $count"
scale=$(seval '(assq-ref (car (output-heads)) (quote scale))')
[ "$scale" = "1.0" ] || fail "initial scale should be 1.0, got $scale"
echo "ok - (output-heads) lists the winit head with modes and state"

# --- configure-output! applies a scale visible to wlr-randr ---------------
r=$(seval '(configure-output! "winit" #:scale 2)') || fail "configure-output! eval failed"
[ "$r" = "#t" ] || fail "configure-output! did not return #t (got $r)"

# The change is queued to the compositor thread; poll wlr-randr.
attempt=0
while :; do
    q=$(nested_wayland timeout 15 wlr-randr 2>"$OUT/wlr-randr.err") ||
        fail "wlr-randr query failed" "$OUT/wlr-randr.err"
    printf '%s\n' "$q" >"$OUT/query.txt"
    s=$(printf '%s' "$q" | sed -n 's/.*Scale:[[:space:]]*\([0-9.]*\).*/\1/p' | head -n1)
    is_two=$(awk -v s="${s:-0}" 'BEGIN { print (s+0 == 2) ? "1" : "0" }')
    [ "$is_two" = "1" ] && break
    attempt=$((attempt + 1))
    [ "$attempt" -le 10 ] || fail "wlr-randr never showed Scale 2 (got '$s')" "$OUT/query.txt"
    sleep 1
done
echo "ok - (configure-output! \"winit\" #:scale 2) reached wlr-randr (Scale=$s)"

scale=$(seval '(assq-ref (car (output-heads)) (quote scale))')
[ "$scale" = "2.0" ] || fail "(output-heads) scale should be 2.0 after apply, got $scale"
echo "ok - (output-heads) reflects the new scale"

# --- rejections reach handle-output-configure-failed! ---------------------
seval '(begin
         (define %e2e-failures (quote ()))
         (define (handle-output-configure-failed! name reason)
           (set! %e2e-failures (cons (list name reason) %e2e-failures)))
         (quote installed))' >/dev/null ||
    fail "could not install the failure hook"
# winit cannot change modes: refused, head untouched.
r=$(seval '(configure-output! "winit" #:mode "640x480@60")')
[ "$r" = "#t" ] || fail "mode-change request was not queued (got $r)"
# Unknown head: refused too.
seval '(configure-output! "HDMI-A-9" #:scale 1)' >/dev/null
attempt=0
while :; do
    n=$(seval '(length %e2e-failures)')
    [ "$n" = "2" ] && break
    attempt=$((attempt + 1))
    [ "$attempt" -le 10 ] || fail "expected 2 rejections reported to handle-output-configure-failed!, got $n"
    sleep 1
done
failures=$(seval '%e2e-failures')
printf '%s\n' "$failures" >"$OUT/failures.txt"
printf '%s' "$failures" | grep -q '"winit"' ||
    fail "mode-change rejection did not name the winit head" "$OUT/failures.txt"
printf '%s' "$failures" | grep -q '"HDMI-A-9"' ||
    fail "unknown-head rejection did not name the head" "$OUT/failures.txt"
scale=$(seval '(assq-ref (car (output-heads)) (quote scale))')
[ "$scale" = "2.0" ] || fail "a rejected change altered the head (scale now $scale)"
echo "ok - refused changes are reported to handle-output-configure-failed! and leave the head alone"

# --- a malformed setting is a Scheme error, not a silent no-op ------------
if seval '(configure-output! "winit" #:scale "big")' >"$OUT/malformed.log" 2>&1; then
    fail "malformed #:scale was accepted silently" "$OUT/malformed.log"
fi
grep -q 'scale' "$OUT/malformed.log" ||
    fail "the malformed-setting error does not name the setting" "$OUT/malformed.log"
echo "ok - a malformed setting is reported as an error"

kill -0 "$NESTED_WM_PID" 2>/dev/null ||
    fail "compositor died during Scheme output configuration"

echo "output-scheme-e2e: all checks passed"
