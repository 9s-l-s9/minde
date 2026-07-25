#!/bin/sh
# SPDX-License-Identifier: GPL-3.0-or-later
# Bounded nested e2e gate for touchscreen input (wl_touch).
#
# The nested harness has no virtual touchscreen, so real TouchDown/Up/Motion
# streams cannot be synthesized here -- the coordinate transform math is
# covered by a Rust unit test (map_absolute_to_global in src/input.rs) and the
# event routing/lock-gating mirrors the pointer arms. What is assertable
# nested is that the seat advertises the wl_touch capability: `seat.add_touch()`
# runs on both backends, and the winit backend also synthesizes touch events,
# so wl_touch appears in the seat's capability set. wayland-info reports the
# wl_seat capabilities; this gate asserts "touch" is among them. Every client
# call is wrapped in `timeout`.
set -eu
cd "$(dirname "$0")/.."
. tests/lib/nested-compositor.sh

OUT=${MINDE_TOUCH_E2E_OUT:-/tmp/swm16a}
trap nested_stop EXIT HUP INT TERM

command -v wayland-info >/dev/null 2>&1 || {
    echo "error: wayland-info is required (add wayland-utils)" >&2
    exit 127
}

nested_start "$OUT" "${MINDE_TOUCH_E2E_DISPLAY:-:99}" || {
    echo "error: nested compositor failed; inspect $OUT" >&2
    exit 1
}

info=$(nested_wayland timeout 15 wayland-info 2>"$OUT/wayland-info.err") || {
    echo "error: wayland-info failed against the nested compositor" >&2
    cat "$OUT/wayland-info.err" >&2 || true
    exit 1
}
printf '%s\n' "$info" >"$OUT/wayland-info.txt"

# wayland-info prints the wl_seat block with a "capabilities:" line listing
# pointer/keyboard/touch. Assert touch is advertised.
printf '%s' "$info" | grep -A6 "interface: 'wl_seat'" | grep -qi "touch" || {
    echo "error: compositor did not advertise the wl_touch seat capability" >&2
    cat "$OUT/wayland-info.txt" >&2 || true
    exit 1
}
echo "ok - wl_touch seat capability advertised"

kill -0 "$NESTED_WM_PID" 2>/dev/null || {
    echo "error: compositor died during the touch checks" >&2
    exit 1
}
echo "ok - compositor stable"

echo "touch-e2e: all checks passed"
