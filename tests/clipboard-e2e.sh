#!/bin/sh
# SPDX-License-Identifier: GPL-3.0-or-later
# Bounded nested e2e gate for the sprint-14 clipboard ecosystem:
#   * zwlr_data_control_manager_v1 and ext_data_control_manager_v1
#     (clipboard managers), and
#   * zwp_primary_selection_device_manager_v1 (middle-click paste).
# Asserts the manager globals are advertised (wayland-info) and that both the
# regular clipboard and the primary selection round-trip through wl-clipboard.
# Uses the shared nested-compositor lifecycle so socket paths stay < SUN_LEN
# and teardown is uniform. Every client call is wrapped in `timeout`.
set -eu
cd "$(dirname "$0")/.."
. tests/lib/nested-compositor.sh

OUT=${MINDE_CLIPBOARD_E2E_OUT:-/tmp/swm-clip-e2e}
trap nested_stop EXIT HUP INT TERM

for tool in wl-copy wl-paste wayland-info; do
    command -v "$tool" >/dev/null 2>&1 || {
        echo "error: $tool is required (add wl-clipboard / wayland-utils)" >&2
        exit 127
    }
done

nested_start "$OUT" "${MINDE_CLIPBOARD_E2E_DISPLAY:-:97}" || {
    echo "error: nested compositor failed; inspect $OUT" >&2
    exit 1
}

# --- manager globals advertised -------------------------------------------
info=$(nested_wayland timeout 15 wayland-info 2>"$OUT/wayland-info.err") || {
    echo "error: wayland-info failed against the nested compositor" >&2
    cat "$OUT/wayland-info.err" >&2 || true
    exit 1
}
printf '%s\n' "$info" >"$OUT/wayland-info.txt"

for iface in \
    zwp_primary_selection_device_manager_v1 \
    zwlr_data_control_manager_v1 \
    ext_data_control_manager_v1; do
    printf '%s' "$info" | grep -q "$iface" || {
        echo "error: $iface global is not advertised" >&2
        exit 1
    }
    echo "ok - $iface advertised"
done

# --- regular clipboard round trip -----------------------------------------
clip_text="minde-clipboard-$$"
nested_wayland timeout 10 wl-copy "$clip_text" || {
    echo "error: wl-copy (clipboard) failed" >&2
    exit 1
}
got=$(nested_wayland timeout 10 wl-paste --no-newline 2>"$OUT/paste.err") || {
    echo "error: wl-paste (clipboard) failed" >&2
    cat "$OUT/paste.err" >&2 || true
    exit 1
}
[ "$got" = "$clip_text" ] || {
    echo "error: clipboard round trip mismatch: got '$got' want '$clip_text'" >&2
    exit 1
}
echo "ok - clipboard round trip via wl-clipboard"

# --- primary selection round trip -----------------------------------------
prim_text="minde-primary-$$"
nested_wayland timeout 10 wl-copy --primary "$prim_text" || {
    echo "error: wl-copy --primary failed" >&2
    exit 1
}
got=$(nested_wayland timeout 10 wl-paste --primary --no-newline 2>"$OUT/paste-primary.err") || {
    echo "error: wl-paste --primary failed" >&2
    cat "$OUT/paste-primary.err" >&2 || true
    exit 1
}
[ "$got" = "$prim_text" ] || {
    echo "error: primary selection round trip mismatch: got '$got' want '$prim_text'" >&2
    exit 1
}
echo "ok - primary selection round trip via wl-clipboard"

echo "clipboard-e2e: all checks passed"
