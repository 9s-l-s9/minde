#!/bin/sh
# SPDX-License-Identifier: GPL-3.0-or-later
# Bounded nested e2e gate for wlr-output-management-unstable-v1. Uses wlr-randr
# (the canonical wlr-output-management client, as kanshi/wdisplays do) to query
# the head and then apply a scale change, confirming the change is reflected
# back -- i.e. the query, configuration-apply and re-advertise paths all work
# and reconcile with the compositor's output state. Mode changes are not
# exercised: under the nested winit backend the output size is fixed by the
# host window, so the compositor deliberately fails differing modes. Every
# client call is wrapped in `timeout`.
set -eu
cd "$(dirname "$0")/.."
. tests/lib/nested-compositor.sh

OUT=${MINDE_OUTPUT_MGMT_E2E_OUT:-/tmp/swm-outmgmt-e2e}
trap nested_stop EXIT HUP INT TERM

command -v wlr-randr >/dev/null 2>&1 || {
    echo "error: wlr-randr is required (wlr-output-management client)" >&2
    exit 127
}

nested_start "$OUT" "${MINDE_OUTPUT_MGMT_E2E_DISPLAY:-:99}" || {
    echo "error: nested compositor failed; inspect $OUT" >&2
    exit 1
}

# --- query ----------------------------------------------------------------
q=$(nested_wayland timeout 15 wlr-randr 2>"$OUT/wlr-randr.err") || {
    echo "error: wlr-randr query failed" >&2
    cat "$OUT/wlr-randr.err" >&2 || true
    exit 1
}
printf '%s\n' "$q" >"$OUT/query.txt"

printf '%s' "$q" | grep -q '^winit' || {
    echo "error: wlr-randr did not report the 'winit' head" >&2
    cat "$OUT/query.txt" >&2 || true
    exit 1
}
printf '%s' "$q" | grep -qi 'Modes:' || {
    echo "error: wlr-randr reported no modes for the head" >&2
    exit 1
}
echo "ok - wlr-randr queried the head and its modes"

# --- apply a scale change -------------------------------------------------
nested_wayland timeout 15 wlr-randr --output winit --scale 2 \
    >"$OUT/apply.log" 2>&1 || {
    echo "error: wlr-randr scale apply failed" >&2
    cat "$OUT/apply.log" >&2 || true
    exit 1
}

q2=$(nested_wayland timeout 15 wlr-randr 2>"$OUT/wlr-randr2.err") || {
    echo "error: wlr-randr re-query failed" >&2
    cat "$OUT/wlr-randr2.err" >&2 || true
    exit 1
}
printf '%s\n' "$q2" >"$OUT/query2.txt"

# wlr-randr prints "Scale: 2.000000" (allow any trailing zeros).
scale=$(printf '%s' "$q2" | sed -n 's/.*Scale:[[:space:]]*\([0-9.]*\).*/\1/p' | head -n1)
[ -n "$scale" ] || {
    echo "error: could not read the head scale after apply" >&2
    cat "$OUT/query2.txt" >&2 || true
    exit 1
}
is_two=$(awk -v s="$scale" 'BEGIN { print (s+0 == 2) ? "1" : "0" }')
[ "$is_two" = "1" ] || {
    echo "error: scale did not apply; head reports Scale=$scale, expected 2" >&2
    exit 1
}
echo "ok - wlr-randr applied Scale=2 and the compositor reflected it"

kill -0 "$NESTED_WM_PID" 2>/dev/null || {
    echo "error: compositor died during output configuration" >&2
    exit 1
}

echo "output-management-e2e: all checks passed"
