#!/bin/sh
# SPDX-License-Identifier: GPL-3.0-or-later
# Bounded nested e2e gate for zwp_pointer_constraints_v1 (pointer lock /
# confinement) and zwp_relative_pointer_manager_v1 (raw relative motion).
#
# There is no canonical scriptable pointer-lock CLI client in Guix (the only
# exercisers are interactive: weston's weston-simple-pointer-constraints demo
# and full games), and driving a real lock end to end would need synthesized
# libinput motion the nested winit backend cannot receive. So this gate
# asserts both manager globals are advertised (wayland-info, the same client
# the clipboard and foreign-toplevel gates use); the constraint enforcement
# and region-clamp logic are covered by Rust unit tests in
# src/handlers/pointer_constraints.rs. Every client call is wrapped in
# `timeout`.
set -eu
cd "$(dirname "$0")/.."
. tests/lib/nested-compositor.sh

OUT=${MINDE_POINTER_CONSTRAINTS_E2E_OUT:-/tmp/swm14b}
trap nested_stop EXIT HUP INT TERM

command -v wayland-info >/dev/null 2>&1 || {
    echo "error: wayland-info is required (add wayland-utils)" >&2
    exit 127
}

nested_start "$OUT" "${MINDE_POINTER_CONSTRAINTS_E2E_DISPLAY:-:99}" || {
    echo "error: nested compositor failed; inspect $OUT" >&2
    exit 1
}

info=$(nested_wayland timeout 15 wayland-info 2>"$OUT/wayland-info.err") || {
    echo "error: wayland-info failed against the nested compositor" >&2
    cat "$OUT/wayland-info.err" >&2 || true
    exit 1
}
printf '%s\n' "$info" >"$OUT/wayland-info.txt"

for global in zwp_pointer_constraints_v1 zwp_relative_pointer_manager_v1; do
    printf '%s' "$info" | grep -q "$global" || {
        echo "error: compositor did not advertise $global" >&2
        cat "$OUT/wayland-info.txt" >&2 || true
        exit 1
    }
    echo "ok - $global advertised"
done

kill -0 "$NESTED_WM_PID" 2>/dev/null || {
    echo "error: compositor died during the pointer-constraints checks" >&2
    exit 1
}

echo "pointer-constraints-e2e: all checks passed"
