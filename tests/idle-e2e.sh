#!/bin/sh
# SPDX-License-Identifier: GPL-3.0-or-later
# Bounded nested e2e gate for ext-idle-notify-v1 and the
# zwp_idle_inhibit_manager_v1 (idle-inhibit) globals.
#
# Two layers:
#   1. wayland-info asserts both manager globals are advertised (same client
#      the clipboard/foreign-toplevel/pointer-constraints gates use).
#   2. swayidle (an ext-idle-notify-v1 client) is started with a 1-second
#      idle timeout that touches a marker file. The nested compositor gets no
#      synthetic input, so the seat goes idle and the compositor must fire an
#      `idled` notification; we assert the marker appears. This exercises the
#      real notifier timer end to end, not just global advertisement.
#
# Every client call is wrapped in `timeout`.
set -eu
cd "$(dirname "$0")/.."
. tests/lib/nested-compositor.sh

OUT=${MINDE_IDLE_E2E_OUT:-/tmp/swm14d}
SWAYIDLE_PID=""
cleanup() {
    if [ -n "$SWAYIDLE_PID" ]; then
        kill "$SWAYIDLE_PID" 2>/dev/null || true
    fi
    nested_stop
}
trap cleanup EXIT HUP INT TERM

command -v wayland-info >/dev/null 2>&1 || {
    echo "error: wayland-info is required (add wayland-utils)" >&2
    exit 127
}
command -v swayidle >/dev/null 2>&1 || {
    echo "error: swayidle is required (add swayidle)" >&2
    exit 127
}

nested_start "$OUT" "${MINDE_IDLE_E2E_DISPLAY:-:99}" || {
    echo "error: nested compositor failed; inspect $OUT" >&2
    exit 1
}

info=$(nested_wayland timeout 15 wayland-info 2>"$OUT/wayland-info.err") || {
    echo "error: wayland-info failed against the nested compositor" >&2
    cat "$OUT/wayland-info.err" >&2 || true
    exit 1
}
printf '%s\n' "$info" >"$OUT/wayland-info.txt"

for global in ext_idle_notifier_v1 zwp_idle_inhibit_manager_v1; do
    printf '%s' "$info" | grep -q "$global" || {
        echo "error: compositor did not advertise $global" >&2
        cat "$OUT/wayland-info.txt" >&2 || true
        exit 1
    }
    echo "ok - $global advertised"
done

# Stronger check: swayidle should observe the seat going idle after 1s of no
# input and touch the marker. Run it in the background against the nested
# socket; the marker must appear within a bounded wait.
MARKER="$OUT/idle-marker"
rm -f "$MARKER"
nested_wayland timeout 20 swayidle timeout 1 "touch $MARKER" \
    >"$OUT/swayidle.log" 2>&1 &
SWAYIDLE_PID=$!

fired=0
attempt=0
while [ "$attempt" -lt 15 ]; do
    if [ -e "$MARKER" ]; then
        fired=1
        break
    fi
    kill -0 "$NESTED_WM_PID" 2>/dev/null || {
        echo "error: compositor died during the idle check" >&2
        exit 1
    }
    attempt=$((attempt + 1))
    sleep 1
done

[ "$fired" -eq 1 ] || {
    echo "error: idle notification never fired (marker not created)" >&2
    cat "$OUT/swayidle.log" >&2 || true
    exit 1
}
echo "ok - ext-idle-notify-v1 fired an idle notification (swayidle)"

kill -0 "$NESTED_WM_PID" 2>/dev/null || {
    echo "error: compositor died during the idle checks" >&2
    exit 1
}

echo "idle-e2e: all checks passed"
