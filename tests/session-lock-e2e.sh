#!/bin/sh
# SPDX-License-Identifier: GPL-3.0-or-later
# Security checks use raw protocol clients and synthetic keys in a private
# Xvfb instance. No live desktop, password, or PAM authentication is involved.
set -eu
cd "$(dirname "$0")/.."
. tests/lib/nested-compositor.sh

OUT=${MINDE_SESSION_LOCK_E2E_OUT:-/tmp/minde-session-lock-e2e}
trap nested_stop EXIT HUP INT TERM
for tool in python3 xdotool timeout; do
    command -v "$tool" >/dev/null 2>&1 || {
        echo "error: $tool is required for session-lock-e2e" >&2
        exit 127
    }
done

test_display=${MINDE_SESSION_LOCK_E2E_DISPLAY:-:184}
# Refuse to attach to an existing X server if Xvfb cannot claim this display.
# This also rules out remotely addressed displays and a live host session.
case "$test_display" in
    :*) display_number=${test_display#:} ;;
    *) echo "error: test display must be a local :NUMBER" >&2; exit 1 ;;
esac
case "$display_number" in
    ''|*[!0-9]*) echo "error: test display must be a local :NUMBER" >&2; exit 1 ;;
esac
if [ -e "/tmp/.X11-unix/X$display_number" ] || [ -e "/tmp/.X$display_number-lock" ]; then
    echo "error: X display $test_display is already in use; choose an unused test display" >&2
    exit 1
fi
nested_start "$OUT" "$test_display" || {
    echo "error: nested compositor failed; inspect $OUT" >&2
    exit 1
}

# xdotool sees only the Xvfb created by nested_start. Give its compositor
# window host keyboard focus so XTest keys exercise the actual input path.
window=$(timeout 5 xdotool search --sync --name Smithay | head -n 1)
[ -n "$window" ] || {
    echo "error: private nested compositor window not found" >&2
    exit 1
}
timeout 5 xdotool windowfocus "$window"
export MINDE_SESSION_LOCK_TEST_WINDOW="$window"

if ! timeout 75 python3 tests/clients/session-lock.py >"$OUT/client.log" 2>&1; then
    cat "$OUT/client.log" >&2
    echo "error: session-lock regression failed; inspect $OUT" >&2
    tail -n 40 "$NESTED_LOG" >&2
    exit 1
fi
cat "$OUT/client.log"
kill -0 "$NESTED_WM_PID" 2>/dev/null || {
    echo "error: compositor died during session-lock checks" >&2
    exit 1
}
