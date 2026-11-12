#!/bin/sh
# SPDX-License-Identifier: GPL-3.0-or-later
# Bounded nested gate for xdg map/unmap and foreign-toplevel lifetimes.
# A raw protocol client checks bufferless roles, remaps, focus and grabs;
# foot and wayland-info exercise the real-application announcement path.
set -eu
cd "$(dirname "$0")/.."
. tests/lib/nested-compositor.sh
export MINDE_TEST_PROTOCOLS=1

OUT=${MINDE_FOREIGN_TOPLEVEL_E2E_OUT:-/tmp/swm-ft-e2e}
trap nested_stop EXIT HUP INT TERM

for tool in wayland-info foot xdotool; do
    command -v "$tool" >/dev/null 2>&1 || {
        echo "error: $tool is required for foreign-toplevel-e2e" >&2
        exit 127
    }
done

nested_start "$OUT" "${MINDE_FOREIGN_TOPLEVEL_E2E_DISPLAY:-:98}" || {
    echo "error: nested compositor failed; inspect $OUT" >&2
    exit 1
}

# Direct synthetic keys only to the compositor on this private Xvfb display.
window=$(timeout 5 xdotool search --sync --name Smithay | head -n 1)
[ -n "$window" ] || {
    echo "error: private nested compositor window not found" >&2
    exit 1
}
timeout 5 xdotool windowfocus "$window"

timeout 40 "$NESTED_PROTOCOL_CLIENT" window-lifecycle \
    >"$OUT/window-lifecycle.log" 2>&1 || {
    cat "$OUT/window-lifecycle.log" >&2
    exit 1
}
cat "$OUT/window-lifecycle.log"

# Map a real toplevel so the manager-bind path creates a handle for it.
before=$(nested_window_count 2>/dev/null || printf '0')
scripts/minde-cmd '(wm-spawn "foot")' >/dev/null 2>&1 || true
nested_wait_for_window_after "$before" 20 || {
    echo "error: foot never mapped; cannot exercise foreign-toplevel" >&2
    exit 1
}
echo "ok - a toplevel is mapped"

# Bind the manager with a real window present. wayland-info enumerates every
# global and drains their initial events, so it exercises toplevel/title/
# app_id/state/done for the mapped window.
info=$(nested_wayland timeout 15 wayland-info 2>"$OUT/wayland-info.err") || {
    echo "error: wayland-info failed against the nested compositor" >&2
    cat "$OUT/wayland-info.err" >&2 || true
    exit 1
}
printf '%s\n' "$info" >"$OUT/wayland-info.txt"

printf '%s' "$info" | grep -q zwlr_foreign_toplevel_manager_v1 || {
    echo "error: zwlr_foreign_toplevel_manager_v1 global is not advertised" >&2
    exit 1
}
echo "ok - zwlr_foreign_toplevel_manager_v1 advertised"

# The compositor must still be alive and free of protocol errors after
# streaming the toplevel to the manager.
kill -0 "$NESTED_WM_PID" 2>/dev/null || {
    echo "error: compositor died while serving foreign-toplevel events" >&2
    exit 1
}
if nested_log_has "protocol error"; then
    echo "error: compositor logged a protocol error during the foreign-toplevel scenario" >&2
    exit 1
fi
echo "ok - compositor served foreign-toplevel events cleanly"

echo "foreign-toplevel-e2e: all checks passed"
