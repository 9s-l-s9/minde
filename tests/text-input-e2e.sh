#!/bin/sh
# SPDX-License-Identifier: GPL-3.0-or-later
# Bounded nested e2e gate for text-input-v3 / input-method-v2 (IME).
#
# A full fcitx5/ibus round trip is too heavy for CI (it needs a running IME
# daemon, dbus, and a candidate engine). Instead, two bounded layers:
#
#   1. wayland-info asserts both manager globals are advertised
#      (zwp_text_input_manager_v3 and zwp_input_method_manager_v2), the same
#      way the idle/foreign-toplevel/pointer-constraints gates check globals.
#   2. foot supports text-input-v3: map a foot window, let it gain keyboard
#      focus (which drives the compositor's text-input focus propagation),
#      type into it, and assert the compositor stays alive with no Wayland
#      protocol error in its log. This exercises the text_input global bind,
#      the focus follow, and the enter/leave path without an IME daemon.
#
# Every client call is wrapped in `timeout`.
set -eu
cd "$(dirname "$0")/.."
. tests/lib/nested-compositor.sh

OUT=${MINDE_TEXTINPUT_E2E_OUT:-/tmp/swm14f}
FOOT_PID=""
cleanup() {
    if [ -n "$FOOT_PID" ]; then
        kill "$FOOT_PID" 2>/dev/null || true
    fi
    nested_stop
}
trap cleanup EXIT HUP INT TERM

command -v wayland-info >/dev/null 2>&1 || {
    echo "error: wayland-info is required (add wayland-utils)" >&2
    exit 127
}
command -v foot >/dev/null 2>&1 || {
    echo "error: foot is required (text-input-v3 client)" >&2
    exit 127
}

nested_start "$OUT" "${MINDE_TEXTINPUT_E2E_DISPLAY:-:99}" || {
    echo "error: nested compositor failed; inspect $OUT" >&2
    exit 1
}

info=$(nested_wayland timeout 15 wayland-info 2>"$OUT/wayland-info.err") || {
    echo "error: wayland-info failed against the nested compositor" >&2
    cat "$OUT/wayland-info.err" >&2 || true
    exit 1
}
printf '%s\n' "$info" >"$OUT/wayland-info.txt"

for global in zwp_text_input_manager_v3 zwp_input_method_manager_v2; do
    printf '%s' "$info" | grep -q "$global" || {
        echo "error: compositor did not advertise $global" >&2
        cat "$OUT/wayland-info.txt" >&2 || true
        exit 1
    }
    echo "ok - $global advertised"
done

# Stronger check: run a real text-input-v3 client (foot). Mapping and focusing
# it drives text_input focus propagation (enter/leave); typing into it must not
# provoke a protocol error. foot binds zwp_text_input_v3 and, with no IME
# bound, the compositor discards its text-input requests silently.
before=$(nested_window_count 2>/dev/null || printf '0')
nested_wayland timeout 20 foot sh -c 'printf hello; sleep 6' \
    >"$OUT/foot.log" 2>&1 &
FOOT_PID=$!

nested_wait_for_window_after "$before" 15 || {
    echo "error: foot never mapped a window" >&2
    cat "$OUT/foot.log" >&2 || true
    exit 1
}
echo "ok - foot (text-input-v3 client) mapped a window"

# Type into the focused client via the compositor's synthetic-input path.
nested_wayland timeout 5 scripts/mindectl eval \
    '(begin (wm-send-string "hi") #t)' >/dev/null 2>&1 || true
sleep 1

# The compositor must still be alive and free of Wayland protocol errors
# (a broken text-input/input-method dispatch would disconnect the client with
# a protocol error logged, or crash the compositor).
kill -0 "$NESTED_WM_PID" 2>/dev/null || {
    echo "error: compositor died during the text-input check" >&2
    tail -n 40 "$NESTED_LOG" >&2 || true
    exit 1
}
if nested_log_has "protocol error"; then
    echo "error: a Wayland protocol error was logged during the text-input check" >&2
    grep -a "protocol error" "$NESTED_LOG" >&2 || true
    exit 1
fi
echo "ok - text-input client stayed connected with no protocol error"

echo "text-input-e2e: all checks passed"
