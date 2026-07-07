#!/bin/sh
# SPDX-License-Identifier: GPL-3.0-or-later
# Real input-path smoke test for the shipped reduced keymap.  The broad e2e
# suite intentionally keeps the historical map to cover old behavior.
set -eu
cd "$(dirname "$0")/.."
. tests/lib/nested-compositor.sh

OUT=${MINDE_PORTABLE_E2E_OUT:-/tmp/minde-portable-e2e}
mkdir -p "$OUT"

cleanup() {
    if [ -n "${CLIENT_PID:-}" ]; then
        kill "$CLIENT_PID" 2>/dev/null || true
    fi
    nested_stop
}
trap cleanup EXIT HUP INT TERM

command -v foot >/dev/null 2>&1 || {
    echo "error: foot is required for portable keymap e2e" >&2
    exit 127
}
command -v xdotool >/dev/null 2>&1 || {
    echo "error: xdotool is required for portable keymap e2e" >&2
    exit 127
}

nested_start "$OUT" "${MINDE_PORTABLE_E2E_DISPLAY:-:94}" || {
    echo "error: nested compositor failed; inspect $OUT" >&2
    exit 1
}

before=$(nested_window_count)
nested_wayland timeout 30 foot --app-id=minde-portable-test \
    --title=minde-portable-test sh -c 'sleep 25' >"$OUT/foot.log" 2>&1 &
CLIENT_PID=$!
nested_wait_for_window_after "$before" 12 || {
    echo "error: initial foot window did not map" >&2
    exit 1
}

xdotool search --name Smithay windowfocus >/dev/null

xdotool key Print; sleep 0.2; xdotool key question; sleep 0.5
nested_capture "$OUT/help.png"
xdotool key Escape; sleep 0.2

xdotool key Print; sleep 0.2; xdotool key w; sleep 0.5
nested_capture "$OUT/windows.png"
xdotool key 0; sleep 0.3

xdotool key Print; sleep 0.2; xdotool key w; sleep 0.2; xdotool key w; sleep 0.5
nested_capture "$OUT/window-list.png"
xdotool key Escape; sleep 0.2

# Shift-free directional layers: focus stays on h/j/k/l, while moving and
# exchanging windows live in lowercase nested maps.
xdotool key Print; sleep 0.2; xdotool key f; sleep 0.2; xdotool key h; sleep 0.3
xdotool key Print; sleep 0.2; xdotool key w; sleep 0.2; xdotool key l; sleep 0.3
xdotool key Print; sleep 0.2; xdotool key w; sleep 0.2; xdotool key h; sleep 0.3
xdotool key Print; sleep 0.2; xdotool key f; sleep 0.2; xdotool key x; sleep 0.2; xdotool key l; sleep 0.3
xdotool key Print; sleep 0.2; xdotool key f; sleep 0.2; xdotool key x; sleep 0.2; xdotool key h; sleep 0.3
xdotool key Print; sleep 0.2; xdotool key w; sleep 0.2; xdotool key p; sleep 0.3
xdotool key Print; sleep 0.2; xdotool key w; sleep 0.2; xdotool key u; sleep 0.2; xdotool key p; sleep 0.3

xdotool key Print; sleep 0.2; xdotool key f; sleep 0.5
nested_capture "$OUT/frames.png"
xdotool key 0; sleep 0.3

before=$(nested_window_count)
xdotool key Print; sleep 0.2; xdotool key Return
nested_wait_for_window_after "$before" 12 || {
    echo "error: portable Print Return did not launch a terminal" >&2
    exit 1
}

nested_log_has "error in keybinding" && {
    echo "error: portable keymap produced a keybinding error" >&2
    exit 1
}
for screenshot in help windows window-list frames; do
    [ -s "$OUT/$screenshot.png" ] || {
        echo "error: missing $screenshot screenshot" >&2
        exit 1
    }
done
echo "portable keymap e2e: lowercase submaps, help, numbering, and terminal passed"
