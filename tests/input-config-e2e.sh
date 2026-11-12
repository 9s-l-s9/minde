#!/bin/sh
# SPDX-License-Identifier: GPL-3.0-or-later
# Bounded no-device API smoke test for the libinput configuration surface
# (`wm-input-devices` / `wm-configure-input!`).
#
# The nested backend is winit, which has no libinput context, so this gate
# proves the Scheme surface exists and behaves sanely on the no-device path:
#   - `wm-input-devices` returns the empty list,
#   - `wm-configure-input!` accepts both the `#t`-match and named-match forms
#     with every keyword setting and returns without error.
# Real per-device configuration (tap/scroll/accel/click actually changing a
# touchpad) is hardware owner-verification -- it needs a libinput seat the
# nested compositor cannot provide.
#
# Every eval is driven through the same main-thread IPC (`mindectl eval`)
# the other Scheme gates use, and wrapped in `timeout`.
set -eu
cd "$(dirname "$0")/.."
. tests/lib/nested-compositor.sh

OUT=${MINDE_INPUT_CONFIG_E2E_OUT:-/tmp/swm14-input}
trap nested_stop EXIT HUP INT TERM

nested_start "$OUT" "${MINDE_INPUT_CONFIG_E2E_DISPLAY:-:99}" || {
    echo "error: nested compositor failed; inspect $OUT" >&2
    exit 1
}

ctl_eval() {
    timeout 10 scripts/mindectl eval "$1"
}

devices=$(ctl_eval '(wm-input-devices)') || {
    echo "error: (wm-input-devices) evaluation failed" >&2
    exit 1
}
printf 'wm-input-devices => %s\n' "$devices"
[ "$devices" = "()" ] || {
    echo "error: expected no libinput devices under winit, got: $devices" >&2
    exit 1
}
echo "ok - wm-input-devices is empty on the no-libinput backend"

all=$(ctl_eval '(wm-configure-input! #t #:tap-to-click #t #:natural-scroll #t)') || {
    echo "error: wm-configure-input! (match #t) evaluation failed" >&2
    exit 1
}
[ "$all" = "#t" ] || {
    echo "error: wm-configure-input! (match #t) did not return #t: $all" >&2
    exit 1
}
echo "ok - wm-configure-input! accepts the match-all rule"

named=$(ctl_eval '(wm-configure-input! "Touchpad" #:tap-to-click #t #:natural-scroll #f #:accel-profile (quote adaptive) #:click-method (quote clickfinger) #:map-to-output "eDP-1")') || {
    echo "error: wm-configure-input! (named) evaluation failed" >&2
    exit 1
}
[ "$named" = "#t" ] || {
    echo "error: wm-configure-input! (named) did not return #t: $named" >&2
    exit 1
}
echo "ok - wm-configure-input! accepts a named per-device rule"

auto_mapping=$(ctl_eval '(wm-configure-input! "Touchpad" #:map-to-output (quote auto))')
[ "$auto_mapping" = "#t" ] || { echo "error: automatic output mapping failed: $auto_mapping" >&2; exit 1; }
invalid=$(ctl_eval '(catch #t (lambda () (wm-configure-input! "Touchpad" #:map-to-output 42) #f) (lambda _ #t))')
[ "$invalid" = "#t" ] || { echo "error: invalid output mapping was accepted" >&2; exit 1; }
raw=$(ctl_eval '(wm-configure-input-rule! "Touchpad" -1 -1 "" "" #t)')
[ "$raw" = "#f" ] || { echo "error: primitive accepted a non-string output mapping" >&2; exit 1; }
echo "ok - output mapping accepts automatic selection and rejects invalid values"

# Winit's framebuffer flip must not invert host pointer coordinates. DISPLAY
# belongs to nested_start's private Xvfb, never to the user's desktop.
window=$(timeout 5 xdotool search --sync --name Smithay | head -n 1)
[ -n "$window" ] || { echo "error: private compositor window not found" >&2; exit 1; }
timeout 5 xdotool mousemove --sync --window "$window" 125 75
timeout 5 xdotool mousemove --sync --window "$window" 250 150
attempt=0
while [ "$(ctl_eval '(wm-pointer-position)')" != '(250 150)' ]; do
    attempt=$((attempt + 1))
    [ "$attempt" -lt 10 ] || { echo "error: nested pointer coordinates were transformed" >&2; exit 1; }
    sleep 0.1
done
echo "ok - nested host pointer coordinates ignore the framebuffer flip"

kill -0 "$NESTED_WM_PID" 2>/dev/null || {
    echo "error: compositor died during the input-config checks" >&2
    exit 1
}

echo "input-config-e2e: no-device API checks passed (hardware settings not exercised)"
