#!/bin/sh
# SPDX-License-Identifier: GPL-3.0-or-later
# Bounded nested e2e gate for ext-image-copy-capture-v1: starts the nested
# (winit) compositor and asserts that grim (which speaks
# ext-image-copy-capture-v1, not wlr-screencopy) captures a real,
# correctly-sized frame of the output. Uses the shared nested-compositor
# lifecycle so socket paths stay short (< SUN_LEN) and teardown is uniform.
set -eu
cd "$(dirname "$0")/.."
. tests/lib/nested-compositor.sh

OUT=${MINDE_SCREENCAPTURE_E2E_OUT:-/tmp/swm-e2e}
trap nested_stop EXIT HUP INT TERM

command -v grim >/dev/null 2>&1 || {
    echo "error: grim is required (speaks ext-image-copy-capture-v1)" >&2
    exit 127
}
command -v identify >/dev/null 2>&1 || {
    echo "error: ImageMagick 'identify' is required to validate the PNG" >&2
    exit 127
}

nested_start "$OUT" "${MINDE_SCREENCAPTURE_E2E_DISPLAY:-:96}" || {
    echo "error: nested compositor failed; inspect $OUT" >&2
    exit 1
}

# (wm-outputs) -> ((id x y w h name) ...); ask the running compositor for
# the real mode instead of assuming the Xvfb geometry, so this gate cannot
# silently pass against the wrong dimensions if that ever changes.
outputs=$(nested_wayland scripts/minde-cmd '(wm-outputs)' 2>/dev/null) \
    || { echo "error: wm-outputs query failed" >&2; exit 1; }
expect_w=$(printf '%s' "$outputs" | sed -n 's/.*(0 [0-9-]* [0-9-]* \([0-9]*\) .*/\1/p')
expect_h=$(printf '%s' "$outputs" | sed -n 's/.*(0 [0-9-]* [0-9-]* [0-9]* \([0-9]*\).*/\1/p')
if [ -z "${expect_w:-}" ] || [ -z "${expect_h:-}" ]; then
    echo "error: could not parse output geometry from: $outputs" >&2
    exit 1
fi

png="$OUT/grim-capture.png"
nested_wayland timeout 20 grim "$png" >"$OUT/grim.log" 2>&1 || {
    echo "error: grim exited non-zero capturing ext-image-copy-capture-v1" >&2
    cat "$OUT/grim.log" >&2 || true
    exit 1
}

[ -s "$png" ] || { echo "error: grim produced an empty file" >&2; exit 1; }

identify_out=$(identify -format '%m %w %h %[standard-deviation]' "$png" 2>"$OUT/identify.log") || {
    echo "error: captured file is not a parseable image" >&2
    cat "$OUT/identify.log" >&2 || true
    exit 1
}
fmt=$(printf '%s' "$identify_out" | awk '{print $1}')
w=$(printf '%s' "$identify_out" | awk '{print $2}')
h=$(printf '%s' "$identify_out" | awk '{print $3}')
stddev=$(printf '%s' "$identify_out" | awk '{print $4}')

[ "$fmt" = "PNG" ] || { echo "error: grim did not produce a PNG (got $fmt)" >&2; exit 1; }
if [ "$w" != "$expect_w" ] || [ "$h" != "$expect_h" ]; then
    echo "error: captured image is ${w}x${h}, expected ${expect_w}x${expect_h} (wm-outputs)" >&2
    exit 1
fi

# A capture bug that ships a blank/uninitialized buffer typically renders as
# a single flat color (stddev == 0). The nested compositor's own chrome
# (background plus any decoration) makes a truly flat frame unlikely if the
# capture path is actually reading composited output.
stddev_zero=$(awk -v s="$stddev" 'BEGIN { print (s == 0 || s == "0") ? "1" : "0" }')
if [ "$stddev_zero" = "1" ]; then
    echo "error: captured image is a single flat color (stddev=$stddev); capture buffer looks blank" >&2
    exit 1
fi

echo "ok - grim (ext-image-copy-capture-v1) captured a valid ${w}x${h} PNG (stddev=$stddev)"
echo "screencapture-e2e: all checks passed"
