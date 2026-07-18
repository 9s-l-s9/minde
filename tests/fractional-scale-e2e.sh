#!/bin/sh
# SPDX-License-Identifier: GPL-3.0-or-later
# Bounded nested e2e gate for wp-fractional-scale-v1 and wp-viewporter.
#
# 1. Asserts both globals are advertised via wayland-info.
# 2. Sets a fractional output scale (1.5) with wlr-randr (the canonical
#    wlr-output-management client) and confirms the compositor reflects it and
#    stays alive -- exercising the preferred-scale propagation path.
# 3. Runs foot at that fractional scale and confirms it maps and draws without
#    a protocol error, so a real fractional-scale/viewporter client survives
#    the scale change end to end.
#
# Every client call is wrapped in `timeout`; the whole run uses a short
# XDG_RUNTIME_DIR (via the shared nested lib) and hard per-step limits.
set -eu
cd "$(dirname "$0")/.."
. tests/lib/nested-compositor.sh

OUT=${MINDE_FRACTIONAL_E2E_OUT:-/tmp/swm14c}
trap nested_stop EXIT HUP INT TERM

for tool in wayland-info wlr-randr foot; do
    command -v "$tool" >/dev/null 2>&1 || {
        echo "error: $tool is required" >&2
        exit 127
    }
done

nested_start "$OUT" "${MINDE_FRACTIONAL_E2E_DISPLAY:-:97}" || {
    echo "error: nested compositor failed; inspect $OUT" >&2
    exit 1
}

# --- globals advertised ---------------------------------------------------
info=$(nested_wayland timeout 15 wayland-info 2>"$OUT/wayland-info.err") || {
    echo "error: wayland-info failed" >&2
    cat "$OUT/wayland-info.err" >&2 || true
    exit 1
}
printf '%s\n' "$info" >"$OUT/wayland-info.txt"

printf '%s' "$info" | grep -q 'wp_fractional_scale_manager_v1' || {
    echo "error: wp_fractional_scale_manager_v1 not advertised" >&2
    exit 1
}
printf '%s' "$info" | grep -q 'wp_viewporter' || {
    echo "error: wp_viewporter not advertised" >&2
    exit 1
}
echo "ok - wp-fractional-scale-v1 and wp-viewporter are advertised"

# --- apply a fractional scale ---------------------------------------------
nested_wayland timeout 15 wlr-randr --output winit --scale 1.5 \
    >"$OUT/apply.log" 2>&1 || {
    echo "error: wlr-randr fractional scale apply failed" >&2
    cat "$OUT/apply.log" >&2 || true
    exit 1
}

q=$(nested_wayland timeout 15 wlr-randr 2>"$OUT/wlr-randr.err") || {
    echo "error: wlr-randr re-query failed" >&2
    cat "$OUT/wlr-randr.err" >&2 || true
    exit 1
}
printf '%s\n' "$q" >"$OUT/query.txt"
scale=$(printf '%s' "$q" | sed -n 's/.*Scale:[[:space:]]*\([0-9.]*\).*/\1/p' | head -n1)
is_onepointfive=$(awk -v s="$scale" 'BEGIN { print (s+0 == 1.5) ? "1" : "0" }')
[ "$is_onepointfive" = "1" ] || {
    echo "error: fractional scale did not apply; head reports Scale=$scale, expected 1.5" >&2
    cat "$OUT/query.txt" >&2 || true
    exit 1
}
kill -0 "$NESTED_WM_PID" 2>/dev/null || {
    echo "error: compositor died applying the fractional scale" >&2
    exit 1
}
echo "ok - wlr-randr applied Scale=1.5 and the compositor reflected it and lives"

# --- a real client at the fractional scale --------------------------------
before=$(nested_window_count 2>/dev/null || printf '0')
nested_wayland foot sh -c 'sleep 6' >"$OUT/foot.log" 2>&1 &
FOOT_PID=$!

if nested_wait_for_window_after "$before" 20; then
    echo "ok - foot mapped a window at the fractional scale"
else
    echo "error: foot did not map a window within the timeout" >&2
    cat "$OUT/foot.log" >&2 || true
    kill "$FOOT_PID" 2>/dev/null || true
    exit 1
fi

# foot must not have died on a protocol error.
if grep -qiE 'protocol error|error marshalling|wl_display@1: error' "$OUT/foot.log"; then
    echo "error: foot reported a protocol error at the fractional scale" >&2
    cat "$OUT/foot.log" >&2 || true
    kill "$FOOT_PID" 2>/dev/null || true
    exit 1
fi

kill "$FOOT_PID" 2>/dev/null || true
wait "$FOOT_PID" 2>/dev/null || true

kill -0 "$NESTED_WM_PID" 2>/dev/null || {
    echo "error: compositor died after the fractional-scale client ran" >&2
    exit 1
}

echo "fractional-scale-e2e: all checks passed"
