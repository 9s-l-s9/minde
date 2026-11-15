#!/bin/sh
# SPDX-License-Identifier: GPL-3.0-or-later
# Frame-paced native raster client in a private nested compositor.
set -eu
cd "$(dirname "$0")/.."
. tests/lib/nested-compositor.sh
export MINDE_TEST_PROTOCOLS=1
nested_output_directory "${MINDE_FRAME_CALLBACKS_E2E_OUT:-}" frame-callbacks-e2e
nested_install_cleanup nested_stop
nested_start "$OUT" "${MINDE_FRAME_CALLBACKS_E2E_DISPLAY:-:99}"
export MINDE_TEST_WM_PID="$NESTED_WM_PID"
nested_wayland timeout 45 "$NESTED_PROTOCOL_CLIENT" frame-callbacks >"$OUT/frames.log" 2>&1 || {
    cat "$OUT/frames.log" >&2
    exit 1
}
cat "$OUT/frames.log"
nested_wayland timeout 30 "$NESTED_PROTOCOL_CLIENT" surface-commits >"$OUT/commits.log" 2>&1 || {
    cat "$OUT/commits.log" >&2
    exit 1
}
cat "$OUT/commits.log"
echo "frame-callbacks-e2e: all checks passed"
