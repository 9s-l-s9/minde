#!/bin/sh
# SPDX-License-Identifier: GPL-3.0-or-later
# Bounded nested e2e gate for the libinput configuration surface
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

named=$(ctl_eval '(wm-configure-input! "Touchpad" #:tap-to-click #t #:natural-scroll #f #:accel-profile (quote adaptive) #:click-method (quote clickfinger))') || {
    echo "error: wm-configure-input! (named) evaluation failed" >&2
    exit 1
}
[ "$named" = "#t" ] || {
    echo "error: wm-configure-input! (named) did not return #t: $named" >&2
    exit 1
}
echo "ok - wm-configure-input! accepts a named per-device rule"

kill -0 "$NESTED_WM_PID" 2>/dev/null || {
    echo "error: compositor died during the input-config checks" >&2
    exit 1
}

echo "input-config-e2e: all checks passed"
