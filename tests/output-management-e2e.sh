#!/bin/sh
# SPDX-License-Identifier: GPL-3.0-or-later
# Bounded nested e2e gate for wlr-output-management-unstable-v1. Uses wlr-randr
# (the canonical wlr-output-management client, as kanshi/wdisplays do) to query
# the head and then apply a scale change, confirming the change is reflected
# back -- i.e. the query, configuration-apply and re-advertise paths all work
# and reconcile with the compositor's output state. Mode changes are not
# exercised: under the nested winit backend the output size is fixed by the
# host window, so the compositor deliberately fails differing modes. Also
# covers: honest Enabled/Adaptive Sync reporting, a no-op apply, refusal of
# disabling the last head and of adaptive sync, and (with the test-only
# MINDE_OUTPUT_MGMT_ALLOW_NO_HEADS=1 override) a disable -> enable round
# trip during which the head stays advertised. Every
# client call is wrapped in `timeout`.
set -eu
cd "$(dirname "$0")/.."
. tests/lib/nested-compositor.sh

OUT=${MINDE_OUTPUT_MGMT_E2E_OUT:-/tmp/swm-outmgmt-e2e}
ALLOW_ENV=MINDE_OUTPUT_MGMT_ALLOW_NO_HEADS
# The nested compositor inherits our environment; make sure the override is
# off for the first (normal) run.
unset MINDE_OUTPUT_MGMT_ALLOW_NO_HEADS
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
printf '%s' "$q" | grep -q 'Enabled: yes' || {
    echo "error: wlr-randr did not report the head as enabled" >&2
    cat "$OUT/query.txt" >&2 || true
    exit 1
}
# v4 heads carry an adaptive_sync event; wlr-randr prints it for enabled heads.
printf '%s' "$q" | grep -q 'Adaptive Sync:' || {
    echo "error: wlr-randr did not report an Adaptive Sync line (v4 head event)" >&2
    cat "$OUT/query.txt" >&2 || true
    exit 1
}
echo "ok - wlr-randr queried the head, its modes, Enabled and Adaptive Sync"

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

# --- no-op configuration succeeds ----------------------------------------
nested_wayland timeout 15 wlr-randr --output winit --pos 0,0 \
    >"$OUT/noop.log" 2>&1 || {
    echo "error: no-op configuration (--pos 0,0) was refused" >&2
    cat "$OUT/noop.log" >&2 || true
    exit 1
}
echo "ok - a no-op configuration succeeds"

# --- disabling the last head is refused without the test override --------
if nested_wayland timeout 15 wlr-randr --output winit --off \
    >"$OUT/off-refused.log" 2>&1; then
    echo "error: disabling the only head succeeded without $ALLOW_ENV" >&2
    exit 1
fi
echo "ok - disabling the last enabled head is refused"

# --- adaptive sync: winit cannot do it, so enabling must fail ------------
if nested_wayland timeout 15 wlr-randr --output winit --adaptive-sync enabled \
    >"$OUT/vrr.log" 2>&1; then
    echo "error: --adaptive-sync enabled succeeded on the winit backend" >&2
    exit 1
fi
echo "ok - adaptive sync enable is refused where unsupported"

# --- mode changes: the winit window has a fixed size, so they must fail --
if nested_wayland timeout 15 wlr-randr --output winit --custom-mode 640x480@60 \
    >"$OUT/mode.log" 2>&1; then
    echo "error: --custom-mode succeeded on the winit backend" >&2
    exit 1
fi
echo "ok - a mode change is refused where the backend cannot modeset"

kill -0 "$NESTED_WM_PID" 2>/dev/null || {
    echo "error: compositor died during refused configurations" >&2
    exit 1
}

# --- disable -> enable round trip with the test-only override ------------
# A real session must never lose its last head, so the compositor only
# allows it when MINDE_OUTPUT_MGMT_ALLOW_NO_HEADS=1 was set at startup.
nested_stop
unset NESTED_WM_PID NESTED_XVFB_PID NESTED_RT
export MINDE_OUTPUT_MGMT_ALLOW_NO_HEADS=1
nested_start "$OUT/allow-no-heads" "${MINDE_OUTPUT_MGMT_E2E_DISPLAY:-:99}" || {
    echo "error: nested compositor (allow-no-heads) failed; inspect $OUT" >&2
    exit 1
}

nested_wayland timeout 15 wlr-randr --output winit --off \
    >"$OUT/off.log" 2>&1 || {
    echo "error: disabling the head failed with $ALLOW_ENV=1" >&2
    cat "$OUT/off.log" >&2 || true
    exit 1
}
q3=$(nested_wayland timeout 15 wlr-randr 2>"$OUT/wlr-randr3.err") || {
    echo "error: wlr-randr query after --off failed" >&2
    cat "$OUT/wlr-randr3.err" >&2 || true
    exit 1
}
printf '%s\n' "$q3" >"$OUT/query3.txt"
printf '%s' "$q3" | grep -q '^winit' || {
    echo "error: disabled head vanished from wlr-randr (must stay advertised)" >&2
    cat "$OUT/query3.txt" >&2 || true
    exit 1
}
printf '%s' "$q3" | grep -q 'Enabled: no' || {
    echo "error: head not reported as Enabled: no after --off" >&2
    cat "$OUT/query3.txt" >&2 || true
    exit 1
}
echo "ok - the disabled head stays advertised with Enabled: no"

nested_wayland timeout 15 wlr-randr --output winit --on \
    >"$OUT/on.log" 2>&1 || {
    echo "error: re-enabling the head failed" >&2
    cat "$OUT/on.log" >&2 || true
    exit 1
}
q4=$(nested_wayland timeout 15 wlr-randr 2>"$OUT/wlr-randr4.err") || {
    echo "error: wlr-randr query after --on failed" >&2
    cat "$OUT/wlr-randr4.err" >&2 || true
    exit 1
}
printf '%s\n' "$q4" >"$OUT/query4.txt"
printf '%s' "$q4" | grep -q 'Enabled: yes' || {
    echo "error: head not reported as Enabled: yes after --on" >&2
    cat "$OUT/query4.txt" >&2 || true
    exit 1
}
printf '%s' "$q4" | grep -qi 'Modes:' || {
    echo "error: re-enabled head lost its mode list" >&2
    cat "$OUT/query4.txt" >&2 || true
    exit 1
}
echo "ok - --on restores the head with its modes"

kill -0 "$NESTED_WM_PID" 2>/dev/null || {
    echo "error: compositor died during disable/enable" >&2
    exit 1
}

echo "output-management-e2e: all checks passed"
