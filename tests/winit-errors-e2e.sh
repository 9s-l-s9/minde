#!/bin/sh
# SPDX-License-Identifier: GPL-3.0-or-later
set -eu
cd "$(dirname "$0")/.."
. tests/lib/nested-compositor.sh
export MINDE_TEST_PROTOCOLS=1
nested_output_directory "${MINDE_WINIT_ERRORS_E2E_OUT:-}" winit-errors-e2e
nested_install_cleanup nested_stop
mkdir -p "$OUT/lib"
export MINDE_TEST_REAL_EGL
MINDE_TEST_REAL_EGL=$(readlink -f "$(pkg-config --variable=libdir egl)/libEGL.so.1")
# pkg-config compiler flags intentionally split into separate arguments.
# shellcheck disable=SC2046
gcc -shared -fPIC -Wall -Wextra -Werror $(pkg-config --cflags egl) \
    tests/clients/egl-fault.c -ldl -o "$OUT/lib/libEGL.so.1"
export MINDE_NESTED_LIBRARY_PATH="$OUT/lib"
export MINDE_TEST_EGL_FAULT="$OUT/fault"
# The nested backend has one head; use the existing test-only override to
# exercise its disabled-output rendering path as well as DPMS off.
export MINDE_OUTPUT_MGMT_ALLOW_NO_HEADS=1
rm -f "$MINDE_TEST_EGL_FAULT"

for stage in ${MINDE_WINIT_ERROR_STAGES:-render swap}; do
    nested_start "$OUT/$stage" "${MINDE_WINIT_ERRORS_E2E_DISPLAY:-:94}"
    window=$(timeout 5 xdotool search --sync --name Smithay | head -n 1)
    xdotool windowfocus "$window"
    export MINDE_SESSION_LOCK_TEST_WINDOW="$window"
    export MINDE_TEST_COMPOSITOR_LOG="$NESTED_LOG"
    export MINDE_TEST_TARGET_PID="$NESTED_WM_PID"
    timeout 60 "$NESTED_PROTOCOL_CLIENT" winit-errors "$stage"
    attempt=0
    while kill -0 "$NESTED_WM_PID" 2>/dev/null; do
        attempt=$((attempt + 1))
        [ "$attempt" -le 50 ] || { echo 'error: context loss did not stop Minde' >&2; exit 1; }
        sleep 0.1
    done
    wait "$NESTED_WM_PID"
    nested_log_has 'graphics context lost; stopping nested compositor'
    if nested_log_has 'panicked'; then
        echo 'error: nested compositor panicked' >&2
        exit 1
    fi
    nested_stop
done
