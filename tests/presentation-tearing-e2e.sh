#!/bin/sh
# SPDX-License-Identifier: GPL-3.0-or-later
# Bounded nested e2e gate for the timing/sync protocols:
# wp-presentation-time, wp-tearing-control-v1 and linux-drm-syncobj-v1.
#
# Backend coverage (see doc/capability-matrix.md):
#   - wp_tearing_control_manager_v1 : both backends (advisory only). Asserted
#     here via wayland-info against the nested winit compositor.
#   - wp_presentation               : udev-only (needs real vblank clocks).
#     The nested winit backend deliberately does NOT advertise it, so we assert
#     its ABSENCE to lock in the honest udev-only split.
#   - wp_linux_drm_syncobj_manager_v1 : udev-only and gated on a syncobj-capable
#     DRM device, so it can never appear nested -- flagged here for the
#     hardware owner to verify, not asserted.
#
# Every client call is wrapped in `timeout`.
set -eu
cd "$(dirname "$0")/.."
. tests/lib/nested-compositor.sh

OUT=${MINDE_PRESENTATION_E2E_OUT:-/tmp/swm14h}
trap nested_stop EXIT HUP INT TERM

command -v wayland-info >/dev/null 2>&1 || {
    echo "error: wayland-info is required (add wayland-utils)" >&2
    exit 127
}

nested_start "$OUT" "${MINDE_PRESENTATION_E2E_DISPLAY:-:99}" || {
    echo "error: nested compositor failed; inspect $OUT" >&2
    exit 1
}

info=$(nested_wayland timeout 15 wayland-info 2>"$OUT/wayland-info.err") || {
    echo "error: wayland-info failed against the nested compositor" >&2
    cat "$OUT/wayland-info.err" >&2 || true
    exit 1
}
printf '%s\n' "$info" >"$OUT/wayland-info.txt"

printf '%s' "$info" | grep -q "wp_tearing_control_manager_v1" || {
    echo "error: compositor did not advertise wp_tearing_control_manager_v1" >&2
    cat "$OUT/wayland-info.txt" >&2 || true
    exit 1
}
echo "ok - wp_tearing_control_manager_v1 advertised (both backends, advisory)"

# wp_presentation is udev-only; assert it is NOT served on the winit backend so
# the documented split stays honest.
if printf '%s' "$info" | grep -q "wp_presentation"; then
    echo "error: wp_presentation advertised on winit; it is documented udev-only" >&2
    cat "$OUT/wayland-info.txt" >&2 || true
    exit 1
fi
echo "ok - wp_presentation not advertised nested (udev-only, as documented)"

# linux-drm-syncobj is udev-only and capability-gated; it cannot appear nested.
if printf '%s' "$info" | grep -q "wp_linux_drm_syncobj_manager_v1"; then
    echo "error: wp_linux_drm_syncobj_manager_v1 advertised on winit; it is udev-only" >&2
    exit 1
fi
echo "note - linux-drm-syncobj-v1 is udev-only; verify on syncobj-capable hardware"

kill -0 "$NESTED_WM_PID" 2>/dev/null || {
    echo "error: compositor died during the presentation/tearing checks" >&2
    exit 1
}

echo "presentation-tearing-e2e: all checks passed"
