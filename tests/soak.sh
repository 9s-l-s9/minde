#!/bin/sh
# SPDX-License-Identifier: GPL-3.0-or-later
# Repeated nested lifecycle exercise.  Default duration is the release-roadmap
# hour; set SOAK_ITERATIONS for a deterministic short/local smoke run.
set -eu
cd "$(dirname "$0")/.."
. tests/lib/nested-compositor.sh

OUT=${MINDE_SOAK_OUT:-/tmp/minde-soak}
MINUTES=${SOAK_MINUTES:-60}
ITERATIONS=${SOAK_ITERATIONS:-0}
MAX_RSS_GROWTH_KIB=${SOAK_MAX_RSS_GROWTH_KIB:-262144}
RESULTS="$OUT/iterations.jsonl"
mkdir -p "$OUT"
: >"$RESULTS"

cleanup() {
    nested_stop
}
trap cleanup EXIT HUP INT TERM

command -v foot >/dev/null 2>&1 || {
    echo "error: foot is required for the map/unmap soak cycle" >&2
    exit 127
}

if ! nested_start "$OUT" "${MINDE_SOAK_DISPLAY:-:95}"; then
    echo "error: nested compositor failed; inspect $OUT" >&2
    exit 1
fi

started_epoch=$(date +%s)
deadline=$((started_epoch + MINUTES * 60))
iteration=0
baseline_rss=$(awk '/VmRSS:/ { print $2 }' "/proc/$NESTED_WM_PID/status")
max_rss=$baseline_rss

while :; do
    [ "$ITERATIONS" -gt 0 ] && [ "$iteration" -ge "$ITERATIONS" ] && break
    [ "$ITERATIONS" -eq 0 ] && [ "$iteration" -gt 0 ] && [ "$(date +%s)" -ge "$deadline" ] && break
    iteration=$((iteration + 1))
    iteration_started=$(date +%s%3N)
    before=$(nested_window_count)

    nested_wayland timeout 15 foot --app-id=minde-soak \
        --title="minde soak $iteration" sh -c 'sleep 10' \
        >"$OUT/foot-$iteration.log" 2>&1 &
    client_pid=$!
    if ! nested_wait_for_window_after "$before" 10; then
        nested_capture "$OUT/failure-$iteration.png"
        echo "error: iteration $iteration did not map foot" >&2
        exit 1
    fi

    scripts/mindectl eval '(reload-configuration!)' >/dev/null
    scripts/mindectl eval \
        '(begin
           (use-modules (minde groups))
           (create-group! " minde-soak ")
           (handle-heads-change! (quote ((0 0 0 640 800) (1 640 0 640 800))))
           (handle-heads-change! (quote ((0 0 0 1280 800))))
           (set-clipboard! "minde soak clipboard")
           (delete-current-group!))' >/dev/null

    state=$(scripts/mindectl query state --json)
    printf '%s\n' "$state" | jq -e '.schema_version == 1 and .groups and .outputs' >/dev/null

    kill "$client_pid" 2>/dev/null || true
    wait "$client_pid" 2>/dev/null || true
    attempt=0
    while [ "$(nested_window_count)" -gt "$before" ]; do
        attempt=$((attempt + 1))
        [ "$attempt" -le 10 ] || {
            nested_capture "$OUT/failure-$iteration.png"
            echo "error: iteration $iteration left a mapped window" >&2
            exit 1
        }
        sleep 1
    done

    kill -0 "$NESTED_WM_PID" 2>/dev/null || {
        echo "error: compositor exited during iteration $iteration" >&2
        exit 1
    }
    nested_log_has "panicked at" && {
        echo "error: compositor panic during iteration $iteration" >&2
        exit 1
    }

    rss=$(awk '/VmRSS:/ { print $2 }' "/proc/$NESTED_WM_PID/status")
    [ "$rss" -gt "$max_rss" ] && max_rss=$rss
    sequence=$(printf '%s\n' "$state" | jq '.sequence')
    duration=$(($(date +%s%3N) - iteration_started))
    jq -cn --argjson iteration "$iteration" --argjson duration_ms "$duration" \
        --argjson rss_kib "$rss" --argjson status_sequence "$sequence" \
        '{schema_version:1,iteration:$iteration,duration_ms:$duration_ms,
          rss_kib:$rss_kib,status_sequence:$status_sequence,
          operations:["map-unmap","reload","group-switch","clipboard","simulated-hotplug","status-query"]}' \
        >>"$RESULTS"
    printf 'ok - soak iteration %s (%sms, %s KiB RSS)\n' "$iteration" "$duration" "$rss"
done

growth=$((max_rss - baseline_rss))
jq -s --argjson baseline_rss_kib "$baseline_rss" --argjson max_rss_kib "$max_rss" \
    --argjson growth_kib "$growth" \
    '{schema_version:1,iterations:length,baseline_rss_kib:$baseline_rss_kib,
      max_rss_kib:$max_rss_kib,growth_kib:$growth_kib,results:.}' \
    "$RESULTS" >"$OUT/results.json"

if [ "$growth" -gt "$MAX_RSS_GROWTH_KIB" ]; then
    echo "error: RSS grew by $growth KiB (limit $MAX_RSS_GROWTH_KIB KiB)" >&2
    exit 1
fi
printf 'soak: %s iterations passed; RSS growth %s KiB\n' "$iteration" "$growth"
