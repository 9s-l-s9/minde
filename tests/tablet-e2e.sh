#!/bin/sh
# SPDX-License-Identifier: GPL-3.0-or-later
# Bounded nested e2e gate for tablet/stylus input (zwp_tablet_manager_v2).
#
# The nested harness has no virtual tablet, so real TabletTool proximity/axis/
# tip/button streams cannot be synthesized here -- the event routing and the
# pointer-emulation fallback decision are covered by Rust unit tests
# (tool_route and map_absolute_to_global in src/input.rs), and the lock-gating
# reuses the pointer/touch Keyboard-only allowlist. What is assertable nested is
# that the compositor advertises the zwp_tablet_manager_v2 global on both
# backends (TabletManagerState::new runs unconditionally), so tablet-aware
# clients find the protocol. wayland-info lists the registry globals; this gate
# asserts zwp_tablet_manager_v2 is present. Every client call is wrapped in
# `timeout`.
set -eu
cd "$(dirname "$0")/.."
. tests/lib/nested-compositor.sh

OUT=${MINDE_TABLET_E2E_OUT:-/tmp/swm16b}
trap nested_stop EXIT HUP INT TERM

command -v wayland-info >/dev/null 2>&1 || {
    echo "error: wayland-info is required (add wayland-utils)" >&2
    exit 127
}

nested_start "$OUT" "${MINDE_TABLET_E2E_DISPLAY:-:99}" || {
    echo "error: nested compositor failed; inspect $OUT" >&2
    exit 1
}

info=$(nested_wayland timeout 15 wayland-info 2>"$OUT/wayland-info.err") || {
    echo "error: wayland-info failed against the nested compositor" >&2
    cat "$OUT/wayland-info.err" >&2 || true
    exit 1
}
printf '%s\n' "$info" >"$OUT/wayland-info.txt"

# wayland-info lists every advertised global by interface name. Assert the
# tablet manager is among them.
printf '%s' "$info" | grep -q "zwp_tablet_manager_v2" || {
    echo "error: compositor did not advertise zwp_tablet_manager_v2" >&2
    cat "$OUT/wayland-info.txt" >&2 || true
    exit 1
}
echo "ok - zwp_tablet_manager_v2 global advertised"

kill -0 "$NESTED_WM_PID" 2>/dev/null || {
    echo "error: compositor died during the tablet checks" >&2
    exit 1
}
echo "ok - compositor stable"

echo "tablet-e2e: all checks passed"
