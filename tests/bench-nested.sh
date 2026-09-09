#!/bin/sh
# SPDX-License-Identifier: GPL-3.0-or-later
# Isolated Xvfb/software-renderer baseline; CPU is percent of one core.
# Run in guix shell -m manifest.scm. Results are measurements, not gates.
set -eu
cd "$(dirname "$0")/.."
. tests/lib/nested-compositor.sh
OUT=${MINDE_BENCH_OUT:-/tmp/minde-bench-nested}
trap nested_stop EXIT HUP INT TERM
nested_start "$OUT" "${MINDE_BENCH_DISPLAY:-:97}"
if [ "${MINDE_BENCH_GAPS:-0}" = 1 ]; then
    scripts/mindectl eval '(begin (configure-gaps! #:inner 5 #:outer 10 #:head 20) (gaps-on!))' >/dev/null
fi
sleep 3
cpu_ticks() { awk '{print $14 + $15}' "/proc/$NESTED_WM_PID/stat"; }
measure_idle() {
    label=$1
    render_before=$(scripts/mindectl eval '(or (assq (quote render) (wm-timing-stats)) (error "render probe missing"))')
    before=$(cpu_ticks)
    start=$(date +%s%N)
    sleep 5
    end=$(date +%s%N)
    after=$(cpu_ticks)
    awk -v ticks="$((after - before))" -v hz="$(getconf CLK_TCK)" \
        -v ns="$((end - start))" -v label="$label" \
        'BEGIN {printf "%s CPU: %.2f%% of one core\n", label, 100*ticks/hz/(ns/1e9)}'
    render_after=$(scripts/mindectl eval '(or (assq (quote render) (wm-timing-stats)) (error "render probe missing"))')
    printf '%s render stats before: %s\n%s render stats after: %s\n' \
        "$label" "$render_before" "$label" "$render_after"
}
{
    measure_idle idle
    nested_wayland timeout 10 wlopm --off winit
    sleep 1
    measure_idle powered-off
    nested_wayland timeout 10 wlopm --on winit
    scripts/mindectl eval '(+ 1 2)'
} >"$OUT/measurements.txt"
cat "$OUT/measurements.txt"
