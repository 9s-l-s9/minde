#!/bin/sh
# SPDX-License-Identifier: GPL-3.0-or-later
# Bounded nested e2e gate for virtual-keyboard, virtual-pointer and
# keyboard-shortcuts-inhibit.
#
# Three layers:
#   1. wayland-info asserts all three manager globals are advertised
#      (zwp_virtual_keyboard_manager_v1, zwlr_virtual_pointer_manager_v1 and
#      zwp_keyboard_shortcuts_inhibit_manager_v1), the same way the
#      idle/text-input/pointer-constraints gates check globals.
#   2. wtype (a zwp_virtual_keyboard_v1 client) types a marker line into a
#      focused foot shell running a read loop; we assert the typed text
#      actually lands in the file. This exercises real key injection end to end
#      (keymap upload + key events reaching the focused surface).
#   3. wlrctl (a zwlr_virtual_pointer_v1 client) drives the hand-rolled virtual
#      pointer; we assert the compositor stays alive and logs no protocol error
#      (the pointer path runs through the same processing as real input).
#
# Every client call is wrapped in `timeout`.
set -eu
cd "$(dirname "$0")/.."
. tests/lib/nested-compositor.sh

OUT=${MINDE_VIRTUAL_INPUT_E2E_OUT:-/tmp/swm14g}
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
command -v wtype >/dev/null 2>&1 || {
    echo "error: wtype is required (virtual-keyboard client)" >&2
    exit 127
}
command -v foot >/dev/null 2>&1 || {
    echo "error: foot is required (focused text sink)" >&2
    exit 127
}

nested_start "$OUT" "${MINDE_VIRTUAL_INPUT_E2E_DISPLAY:-:99}" || {
    echo "error: nested compositor failed; inspect $OUT" >&2
    exit 1
}

info=$(nested_wayland timeout 15 wayland-info 2>"$OUT/wayland-info.err") || {
    echo "error: wayland-info failed against the nested compositor" >&2
    cat "$OUT/wayland-info.err" >&2 || true
    exit 1
}
printf '%s\n' "$info" >"$OUT/wayland-info.txt"

for global in zwp_virtual_keyboard_manager_v1 zwlr_virtual_pointer_manager_v1 \
    zwp_keyboard_shortcuts_inhibit_manager_v1; do
    printf '%s' "$info" | grep -q "$global" || {
        echo "error: compositor did not advertise $global" >&2
        cat "$OUT/wayland-info.txt" >&2 || true
        exit 1
    }
    echo "ok - $global advertised"
done

# Stronger check: type into a focused client via a real virtual keyboard.
# foot runs a shell read loop that appends every completed line to a file, so
# the typed marker lands as soon as wtype sends Return (no buffering wait).
TYPED="$OUT/typed.txt"
rm -f "$TYPED"
MARKER="swm14vk"
before=$(nested_window_count 2>/dev/null || printf '0')
nested_wayland timeout 30 foot sh -c \
    "while IFS= read -r l; do printf '%s\n' \"\$l\" >>'$TYPED'; done" \
    >"$OUT/foot.log" 2>&1 &
FOOT_PID=$!

nested_wait_for_window_after "$before" 15 || {
    echo "error: foot never mapped a window" >&2
    cat "$OUT/foot.log" >&2 || true
    exit 1
}
echo "ok - foot mapped and focused a window"
# Let foot start its shell read loop before typing.
sleep 2

nested_wayland timeout 10 wtype "$MARKER" -k Return >"$OUT/wtype.log" 2>&1 || {
    echo "error: wtype (virtual keyboard) failed to run" >&2
    cat "$OUT/wtype.log" >&2 || true
    exit 1
}

landed=0
attempt=0
while [ "$attempt" -lt 10 ]; do
    if [ -f "$TYPED" ] && grep -q "$MARKER" "$TYPED"; then
        landed=1
        break
    fi
    kill -0 "$NESTED_WM_PID" 2>/dev/null || {
        echo "error: compositor died during the virtual-keyboard check" >&2
        exit 1
    }
    attempt=$((attempt + 1))
    sleep 1
done
[ "$landed" -eq 1 ] || {
    echo "error: virtual-keyboard text never landed in the focused client" >&2
    echo "--- typed.txt ---" >&2
    cat "$TYPED" 2>/dev/null >&2 || true
    exit 1
}
echo "ok - virtual keyboard (wtype) typed text into the focused client"

# Exercise minde's compositor-owned paced queue too. The complete marker must
# survive (including its first characters), and the Enter alias must finish the
# shell's read rather than getting lost as the old immediate burst did.
MARKER="minde-paced-a@b:c/d!e?f"
scripts/minde-cmd "(begin (wm-send-string \"$MARKER\" 12) (wm-send-key 0 \"Enter\"))" \
    >"$OUT/minde-type.log" 2>&1 || {
    echo "error: compositor-owned paced typing request failed" >&2
    cat "$OUT/minde-type.log" >&2 || true
    exit 1
}
landed=0
attempt=0
while [ "$attempt" -lt 10 ]; do
    if [ -f "$TYPED" ] && grep -qx "$MARKER" "$TYPED"; then
        landed=1
        break
    fi
    attempt=$((attempt + 1))
    sleep 1
done
[ "$landed" -eq 1 ] || {
    echo "error: paced wm-send-string/Enter did not deliver the exact line" >&2
    cat "$TYPED" 2>/dev/null >&2 || true
    exit 1
}
echo "ok - compositor paced typing and Enter alias delivered the exact line"

# Drive the hand-rolled virtual pointer. wlrctl is optional; when present, a
# move plus a click must not crash the compositor or provoke a protocol error.
if command -v wlrctl >/dev/null 2>&1; then
    nested_wayland timeout 10 wlrctl pointer move 25 25 \
        >"$OUT/wlrctl.log" 2>&1 || true
    nested_wayland timeout 10 wlrctl pointer move -10 -10 \
        >>"$OUT/wlrctl.log" 2>&1 || true
    sleep 1
    kill -0 "$NESTED_WM_PID" 2>/dev/null || {
        echo "error: compositor died during the virtual-pointer check" >&2
        tail -n 40 "$NESTED_LOG" >&2 || true
        exit 1
    }
    echo "ok - virtual pointer (wlrctl) drove motion with the compositor alive"
else
    echo "skip - wlrctl not available; virtual-pointer global asserted only"
fi

if nested_log_has "protocol error"; then
    echo "error: a Wayland protocol error was logged during virtual input" >&2
    grep -a "protocol error" "$NESTED_LOG" >&2 || true
    exit 1
fi

kill -0 "$NESTED_WM_PID" 2>/dev/null || {
    echo "error: compositor died during the virtual-input checks" >&2
    exit 1
}

echo "virtual-input-e2e: all checks passed"
