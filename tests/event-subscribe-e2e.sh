#!/bin/sh
# SPDX-License-Identifier: GPL-3.0-or-later
# Bounded nested e2e gate for the read-only event push socket
# ($XDG_RUNTIME_DIR/minde-events.sock). A subscriber attached via
# `mindectl subscribe --events` must receive every fired event as one
# s-expression line in real time. We attach a subscriber, map a real toplevel
# to fire the 'new-window hook, and assert the matching line arrives and parses
# back to the documented payload shape. Every client call is wrapped in
# `timeout` so the gate is bounded.
set -eu
cd "$(dirname "$0")/.."
. tests/lib/nested-compositor.sh

OUT=${MINDE_EVENT_SUBSCRIBE_E2E_OUT:-/tmp/swm15c}
trap nested_stop EXIT HUP INT TERM

for tool in foot guile; do
    command -v "$tool" >/dev/null 2>&1 || {
        echo "error: $tool is required (add foot / guile)" >&2
        exit 127
    }
done

nested_start "$OUT" "${MINDE_EVENT_SUBSCRIBE_E2E_DISPLAY:-:97}" || {
    echo "error: nested compositor failed; inspect $OUT" >&2
    exit 1
}

# The event socket must be published by the running compositor.
attempt=0
until [ -S "$NESTED_RT/minde-events.sock" ]; do
    attempt=$((attempt + 1))
    [ "$attempt" -le 20 ] || { echo "error: event socket never appeared" >&2; exit 1; }
    kill -0 "$NESTED_WM_PID" 2>/dev/null || { echo "error: compositor died" >&2; exit 1; }
    sleep 1
done
echo "ok - event socket published"

# Attach a subscriber, streaming every event line to a file. It shares the
# compositor's XDG_RUNTIME_DIR so it finds the same socket.
events_log="$OUT/events.log"
: >"$events_log"
XDG_RUNTIME_DIR="$NESTED_RT" timeout 30 scripts/mindectl subscribe --events \
    >"$events_log" 2>"$OUT/subscribe.err" &
SUBSCRIBER_PID=$!

# Give the subscriber a moment to connect before firing an event.
attempt=0
until [ -s "$OUT/subscribe.err" ] || kill -0 "$SUBSCRIBER_PID" 2>/dev/null; do
    attempt=$((attempt + 1))
    [ "$attempt" -le 10 ] || break
    sleep 1
done
sleep 1

# Map a real toplevel: (minde windows) fires the 'new-window hook, which the
# glue mirrors to the subscriber.
before=$(nested_window_count 2>/dev/null || printf '0')
scripts/minde-cmd '(wm-spawn "foot")' >/dev/null 2>&1 || true
nested_wait_for_window_after "$before" 20 || {
    echo "error: foot never mapped; no event fired" >&2
    kill "$SUBSCRIBER_PID" 2>/dev/null || true
    exit 1
}
echo "ok - a toplevel is mapped"

# Wait for a new-window line to arrive on the subscription.
attempt=0
until grep -aq '^(new-window ' "$events_log"; do
    attempt=$((attempt + 1))
    [ "$attempt" -le 15 ] || {
        echo "error: no (new-window ...) event line arrived" >&2
        cat "$events_log" >&2 || true
        kill "$SUBSCRIBER_PID" 2>/dev/null || true
        exit 1
    }
    sleep 1
done
kill "$SUBSCRIBER_PID" 2>/dev/null || true
wait "$SUBSCRIBER_PID" 2>/dev/null || true
echo "ok - a (new-window ...) event line arrived"

# The line must parse back to a datum whose head is the event name.
line=$(grep -a '^(new-window ' "$events_log" | head -n 1)
printf '%s\n' "$line" | timeout 10 guile -q -c '
  (let ((datum (read (open-input-string (caddr (command-line))))))
    (unless (and (pair? datum) (eq? (car datum) (quote new-window)))
      (exit 1))
    (unless (integer? (cadr datum))
      (exit 1)))' -- "$line" || {
    echo "error: event line did not parse to a new-window datum: $line" >&2
    exit 1
}
echo "ok - the event line parses to a new-window datum"

echo "event-subscribe-e2e: all checks passed"
