#!/bin/sh
# SPDX-License-Identifier: GPL-3.0-or-later
# Bounded nested e2e gate for wlr-output-power-management-unstable-v1. Uses
# wlopm (the canonical client; what swayidle configurations invoke) to read
# the power mode of the nested winit head, switch it off, confirm the
# compositor reports it off while wlr-output-management still lists the head
# as enabled (power is orthogonal to the layout), and switch it back on.
# Every client call is wrapped in `timeout`.
set -eu
cd "$(dirname "$0")/.."
. tests/lib/nested-compositor.sh

OUT=${MINDE_OUTPUT_POWER_E2E_OUT:-/tmp/swm-outpower-e2e}
trap nested_stop EXIT HUP INT TERM

command -v wlopm >/dev/null 2>&1 || {
    echo "error: wlopm is required (wlr-output-power-management client)" >&2
    exit 127
}
command -v wlr-randr >/dev/null 2>&1 || {
    echo "error: wlr-randr is required (wlr-output-management client)" >&2
    exit 127
}

nested_start "$OUT" "${MINDE_OUTPUT_POWER_E2E_DISPLAY:-:99}" || {
    echo "error: nested compositor failed; inspect $OUT" >&2
    exit 1
}

# Inspect the host window too: protocol replies alone cannot detect a
# missing redraw wake-up that leaves the old desktop visible after DPMS off.
assert_host_power_pixels() {
    expected=$1
    attempt=0
    while [ "$attempt" -lt 20 ]; do
        import -window root "$OUT/host-$expected.png"
        mean=$(identify -format '%[mean]' "$OUT/host-$expected.png")
        if awk -v mean="$mean" -v expected="$expected" \
            'BEGIN {exit !((expected == "off") ? mean == 0 : mean > 0)}'; then
            echo "ok - host pixels reflect power $expected"
            return 0
        fi
        attempt=$((attempt + 1))
        sleep 0.1
    done
    echo "error: host pixels never reflected power $expected" >&2
    return 1
}
assert_host_power_pixels on

# wlopm prints one "<name> <on|off>" line per output.
power_query() {
    nested_wayland timeout 15 wlopm 2>"$OUT/wlopm.err"
}

# --- initial state: on -----------------------------------------------------
q=$(power_query) || {
    echo "error: wlopm query failed" >&2
    cat "$OUT/wlopm.err" >&2 || true
    exit 1
}
printf '%s\n' "$q" >"$OUT/query.txt"
printf '%s' "$q" | grep -q '^winit on$' || {
    echo "error: wlopm did not report 'winit on' initially" >&2
    cat "$OUT/query.txt" >&2 || true
    exit 1
}
echo "ok - wlopm lists the winit head powered on"

# --- off -------------------------------------------------------------------
nested_wayland timeout 15 wlopm --off winit >"$OUT/off.log" 2>&1 || {
    echo "error: wlopm --off winit failed" >&2
    cat "$OUT/off.log" >&2 || true
    exit 1
}
q2=$(power_query) || {
    echo "error: wlopm query after --off failed" >&2
    cat "$OUT/wlopm.err" >&2 || true
    exit 1
}
printf '%s\n' "$q2" >"$OUT/query2.txt"
printf '%s' "$q2" | grep -q '^winit off$' || {
    echo "error: wlopm did not report 'winit off' after --off" >&2
    cat "$OUT/query2.txt" >&2 || true
    exit 1
}
echo "ok - wlopm --off winit: head reports off"
assert_host_power_pixels off

# Power is orthogonal to the layout: the head stays enabled for
# wlr-output-management and the compositor keeps answering.
r=$(nested_wayland timeout 15 wlr-randr 2>"$OUT/wlr-randr.err") || {
    echo "error: wlr-randr query failed while the head is off" >&2
    cat "$OUT/wlr-randr.err" >&2 || true
    exit 1
}
printf '%s\n' "$r" >"$OUT/randr.txt"
printf '%s' "$r" | grep -q '^winit' || {
    echo "error: wlr-randr lost the winit head while powered off" >&2
    cat "$OUT/randr.txt" >&2 || true
    exit 1
}
printf '%s' "$r" | grep -q 'Enabled: yes' || {
    echo "error: powered-off head must stay Enabled: yes for wlr-randr" >&2
    cat "$OUT/randr.txt" >&2 || true
    exit 1
}
echo "ok - the powered-off head stays enabled for wlr-output-management"

kill -0 "$NESTED_WM_PID" 2>/dev/null || {
    echo "error: compositor died while the output was powered off" >&2
    exit 1
}

# --- on --------------------------------------------------------------------
nested_wayland timeout 15 wlopm --on winit >"$OUT/on.log" 2>&1 || {
    echo "error: wlopm --on winit failed" >&2
    cat "$OUT/on.log" >&2 || true
    exit 1
}
q3=$(power_query) || {
    echo "error: wlopm query after --on failed" >&2
    cat "$OUT/wlopm.err" >&2 || true
    exit 1
}
printf '%s\n' "$q3" >"$OUT/query3.txt"
printf '%s' "$q3" | grep -q '^winit on$' || {
    echo "error: wlopm did not report 'winit on' after --on" >&2
    cat "$OUT/query3.txt" >&2 || true
    exit 1
}
echo "ok - wlopm --on winit: head reports on again"
assert_host_power_pixels on

kill -0 "$NESTED_WM_PID" 2>/dev/null || {
    echo "error: compositor died during the power cycle" >&2
    exit 1
}

echo "output-power-e2e: all checks passed"
