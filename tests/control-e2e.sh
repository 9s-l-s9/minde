#!/bin/sh
# SPDX-License-Identifier: GPL-3.0-or-later
# Exercise discovery -> inspect -> act -> verify on isolated real Wayland clients.
set -eu
cd "$(dirname "$0")/.."
. tests/lib/nested-compositor.sh

control_out=${MINDE_CONTROL_E2E_OUT:-/tmp/minde-control-e2e}
control_first=
control_second=
control_cleanup() {
    for control_pid in "$control_first" "$control_second"; do
        if [ -n "$control_pid" ]; then
            kill "$control_pid" 2>/dev/null || true
            wait "$control_pid" 2>/dev/null || true
        fi
    done
    nested_stop
}
trap control_cleanup EXIT HUP INT TERM
nested_start "$control_out" "${MINDE_CONTROL_E2E_DISPLAY:-:87}" || {
    echo "error: control compositor failed; inspect $control_out" >&2
    exit 1
}
nested_wayland timeout 90 foot --app-id=control-one --title='Same title' sh -c 'sleep 80' \
    >"$control_out/first.log" 2>&1 &
control_first=$!
nested_wait_for_window_after 0 20
scripts/mindectl call switch-group! '((group . " II "))' --json >"$control_out/switch.json"
nested_wayland timeout 90 foot --app-id=control-two --title='Same title' sh -c 'sleep 80' \
    >"$control_out/second.log" 2>&1 &
control_second=$!
nested_wait_for_window_after 1 20
timeout 60 python3 tests/control-e2e.py "$control_out"
