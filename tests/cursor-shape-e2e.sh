#!/bin/sh
# SPDX-License-Identifier: GPL-3.0-or-later
# Bounded nested e2e gate for wp_cursor_shape_manager_v1 (cursor-shape-v1)
# and Xcursor theme loading.
#
# There is no canonical scriptable CLI client that sets a cursor shape and
# reports the resulting image (real exercisers are toolkits/games choosing a
# shape on pointer focus). Driving a themed cursor end to end would also need
# the nested winit backend to render its own on-screen cursor, which it does
# not -- the host compositor draws the pointer, so the themed cursor only
# appears in screen captures. So this gate asserts the manager global is
# advertised (wayland-info, the same client the other Sprint 14 gates use)
# and that the compositor starts cleanly under an explicit XCURSOR_THEME /
# XCURSOR_SIZE environment (exercising the theme-load and fallback paths in
# `crate::render::XCursorLoader`). Shape-name mapping and integer size
# selection are covered by Rust unit tests in src/render.rs. Every client
# call is wrapped in `timeout`.
set -eu
cd "$(dirname "$0")/.."
. tests/lib/nested-compositor.sh

OUT=${MINDE_CURSOR_SHAPE_E2E_OUT:-/tmp/swm14g}
trap nested_stop EXIT HUP INT TERM

command -v wayland-info >/dev/null 2>&1 || {
    echo "error: wayland-info is required (add wayland-utils)" >&2
    exit 127
}

# A non-default theme name and size drive the env-configured load path; an
# absent theme must fall back to the built-in bitmap without failing the
# compositor.
export XCURSOR_THEME="${MINDE_CURSOR_SHAPE_E2E_THEME:-Adwaita}"
export XCURSOR_SIZE="${MINDE_CURSOR_SHAPE_E2E_SIZE:-32}"

nested_start "$OUT" "${MINDE_CURSOR_SHAPE_E2E_DISPLAY:-:99}" || {
    echo "error: nested compositor failed; inspect $OUT" >&2
    exit 1
}

info=$(nested_wayland timeout 15 wayland-info 2>"$OUT/wayland-info.err") || {
    echo "error: wayland-info failed against the nested compositor" >&2
    cat "$OUT/wayland-info.err" >&2 || true
    exit 1
}
printf '%s\n' "$info" >"$OUT/wayland-info.txt"

printf '%s' "$info" | grep -q "wp_cursor_shape_manager_v1" || {
    echo "error: compositor did not advertise wp_cursor_shape_manager_v1" >&2
    cat "$OUT/wayland-info.txt" >&2 || true
    exit 1
}
echo "ok - wp_cursor_shape_manager_v1 advertised"

kill -0 "$NESTED_WM_PID" 2>/dev/null || {
    echo "error: compositor died during the cursor-shape checks" >&2
    exit 1
}
echo "ok - compositor stable under XCURSOR_THEME=$XCURSOR_THEME XCURSOR_SIZE=$XCURSOR_SIZE"

echo "cursor-shape-e2e: all checks passed"
