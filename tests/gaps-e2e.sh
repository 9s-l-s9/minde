#!/bin/sh
# SPDX-License-Identifier: GPL-3.0-or-later
# Verify real background pixels between tiled clients in an isolated session.
set -eu
cd "$(dirname "$0")/.."
. tests/lib/nested-compositor.sh
OUT=${MINDE_GAPS_E2E_OUT:-/tmp/minde-gaps-e2e}
FOOT_PID=""
BACKGROUND_PID=""
cleanup() {
    [ -z "$FOOT_PID" ] || kill "$FOOT_PID" 2>/dev/null || true
    [ -z "$BACKGROUND_PID" ] || kill "$BACKGROUND_PID" 2>/dev/null || true
    nested_stop
}
trap cleanup EXIT HUP INT TERM
nested_start "$OUT" "${MINDE_GAPS_E2E_DISPLAY:-:92}"
nested_wayland swaybg -c '#ff0000' >"$OUT/background.log" 2>&1 &
BACKGROUND_PID=$!
before=$(nested_window_count)
nested_wayland env WAYLAND_DEBUG=1 foot -o resize-by-cells=no -o resize-keep-grid=no \
    -o colors.background=000000 sh -c 'while :; do sleep 60; done' >"$OUT/foot.log" 2>&1 &
FOOT_PID=$!
nested_wait_for_window_after "$before" 15
scripts/mindectl eval '(begin (configure-gaps! #:inner 5 #:outer 10 #:head 20) (gaps-on!) (wm-border-color "#00ff00"))' >/dev/null
# Poll pixels to wait for the client configure/commit and the background map.
assert_pixel() {
    label=$1 x=$2 y=$3 expected=$4
    attempt=0
    while [ "$attempt" -lt 30 ]; do
        nested_wayland timeout 10 grim "$OUT/$label.png"
        actual=$(convert "$OUT/$label.png" -format "%[hex:p{$x,$y}]" info:)
        if [ "$actual" = "$expected" ]; then
            echo "ok - $label"
            return 0
        fi
        attempt=$((attempt + 1))
        sleep 0.1
    done
    echo "error: $label pixel was $actual, expected $expected" >&2
    return 1
}
assert_pixel outer-gap 10 10 FF0000
assert_pixel client-content 100 100 000000
# A single frame keeps its gap. Its focus border starts at head+inner+outer.
assert_pixel focus-border 35 40 00FF00
scripts/mindectl eval '(split-frame-horizontal!)' >/dev/null
# Split midpoint is also the midpoint of the symmetric head inset.
width=$(scripts/mindectl eval '(cadddr (car (heads)))')
mid=$((20 + width / 2))
assert_pixel inner-gap "$mid" 100 FF0000
scripts/mindectl eval '(begin (collapse-to-one-frame!) (gaps-off!))' >/dev/null
assert_pixel disabled-restores-client 10 100 000000
scripts/mindectl eval '(gaps-on!)' >/dev/null
assert_pixel enabled-restores-gap 10 10 FF0000
# Check protocol geometry as well as pixels; the renderer must not fake padding.
geometry=$(scripts/mindectl eval '(let ((r (car (heads)))) (list (- (list-ref r 3) 36) (- (list-ref r 4) 36)))')
read -r client_width client_height <<EOF
$(printf '%s' "$geometry" | tr -d '()')
EOF
grep -Eq "xdg_toplevel.*configure\\($client_width, $client_height," "$OUT/foot.log"
nested_wayland timeout 10 wlr-randr --output winit --scale 1.5
assert_pixel scaled-focus-border 53 60 00FF00
assert_pixel scaled-outer-gap 30 30 FF0000
echo 'gaps e2e: all checks passed'
