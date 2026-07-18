#!/bin/sh
# SPDX-License-Identifier: GPL-3.0-or-later
# Bounded nested e2e gate for wlr-foreign-toplevel-management. Guix packages no
# dedicated foreign-toplevel CLI (taskbars are GUI apps), so this gate proves
# the integration surface non-interactively: with a real toplevel mapped, a
# client that binds zwlr_foreign_toplevel_manager_v1 (wayland-info) must
# receive the toplevel/title/app_id/state/done stream without the compositor
# erroring -- i.e. the per-manager handle-creation path runs against a live
# window. Combined with the Rust and Scheme unit tests for the request
# handlers. Every client call is wrapped in `timeout`.
set -eu
cd "$(dirname "$0")/.."
. tests/lib/nested-compositor.sh

OUT=${MINDE_FOREIGN_TOPLEVEL_E2E_OUT:-/tmp/swm-ft-e2e}
trap nested_stop EXIT HUP INT TERM

for tool in wayland-info foot; do
    command -v "$tool" >/dev/null 2>&1 || {
        echo "error: $tool is required (add wayland-utils / foot)" >&2
        exit 127
    }
done

nested_start "$OUT" "${MINDE_FOREIGN_TOPLEVEL_E2E_DISPLAY:-:98}" || {
    echo "error: nested compositor failed; inspect $OUT" >&2
    exit 1
}

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
if nested_log_has "foreign" && nested_log_has "protocol error"; then
    echo "error: compositor logged a foreign-toplevel protocol error" >&2
    exit 1
fi
echo "ok - compositor served foreign-toplevel events cleanly"

echo "foreign-toplevel-e2e: all checks passed"
