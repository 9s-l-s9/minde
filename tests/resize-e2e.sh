#!/bin/sh
# SPDX-License-Identifier: GPL-3.0-or-later
# Exercise the real xdg resize grab and release with a Wayland client.
set -eu
cd "$(dirname "$0")/.."
. tests/lib/nested-compositor.sh
OUT=${MINDE_RESIZE_E2E_OUT:-/tmp/minde-resize-e2e}
FOOT_PID=""
cleanup() {
    [ -z "$FOOT_PID" ] || kill "$FOOT_PID" 2>/dev/null || true
    nested_stop
}
trap cleanup EXIT HUP INT TERM
nested_start "$OUT" "${MINDE_RESIZE_E2E_DISPLAY:-:93}"
before=$(nested_window_count)
nested_wayland env WAYLAND_DEBUG=1 foot -o resize-by-cells=no -o resize-keep-grid=no sh -c 'while :; do sleep 60; done' \
    >"$OUT/foot.log" 2>&1 &
FOOT_PID=$!
nested_wait_for_window_after "$before" 15
scripts/mindectl eval '(begin (float-this!) (set-float-geometry! (focused-window-id) (list 100 100 640 400)) (sync-frames!))' >/dev/null
sleep 1
xdotool search --name Smithay windowfocus
xdotool mousemove 400 300 keydown Super_L mousedown 3
# Multiple host motions plus release in one command exercise the final flush.
xdotool mousemove 410 310 mousemove 420 320 mousemove 430 330 mousemove 460 400 mouseup 3 keyup Super_L
attempt=0
while [ "$attempt" -lt 30 ]; do
    geometry=$(scripts/mindectl eval '(float-geometry (focused-window-id))')
    if [ "$geometry" = '(100 100 700 500)' ] && \
        grep -Eq 'xdg_toplevel.*configure\(700, 500,' "$OUT/foot.log"; then
        break
    fi
    attempt=$((attempt + 1))
    sleep 0.1
done
[ "$attempt" -lt 30 ] || {
    echo "error: resize release geometry: $geometry (expected 100 100 700 500)" >&2
    exit 1
}
# Allow any stale idle callback to run, then check the final size stays put.
sleep 0.3
[ "$(scripts/mindectl eval '(float-geometry (focused-window-id))')" = '(100 100 700 500)' ]
echo 'ok - real xdg resize and release delivered the final size to client and policy'
xdotool mousemove 400 300 keydown Super_L mousedown 3 mousemove 420 320
attempt=0
while ! grep -Eq 'xdg_toplevel.*configure\(720, 520,' "$OUT/foot.log"; do
    attempt=$((attempt + 1))
    [ "$attempt" -lt 30 ] || {
        echo 'error: held resize did not reach the client before release' >&2
        exit 1
    }
    sleep 0.1
done
xdotool mouseup 3 keyup Super_L
attempt=0
while [ "$(scripts/mindectl eval '(float-geometry (focused-window-id))')" != '(100 100 720 520)' ]; do
    attempt=$((attempt + 1))
    [ "$attempt" -lt 30 ] || exit 1
    sleep 0.1
done
echo 'ok - held resize reached the client before release and preserved final geometry'
