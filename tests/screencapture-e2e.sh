#!/bin/sh
# SPDX-License-Identifier: GPL-3.0-or-later
# Bounded nested e2e gate for ext-image-copy-capture-v1: starts the nested
# (winit) compositor and asserts that grim (which speaks
# ext-image-copy-capture-v1, not wlr-screencopy) captures a real,
# correctly-sized frame of the output. Uses the shared nested-compositor
# lifecycle so socket paths stay short (< SUN_LEN) and teardown is uniform.
set -eu
cd "$(dirname "$0")/.."
. tests/lib/nested-compositor.sh
export MINDE_TEST_PROTOCOLS=1

OUT=${MINDE_SCREENCAPTURE_E2E_OUT:-/tmp/swm-e2e}
nested_install_cleanup nested_stop
export MINDE_OUTPUT_MGMT_ALLOW_NO_HEADS=1

command -v grim >/dev/null 2>&1 || {
    echo "error: grim is required (speaks ext-image-copy-capture-v1)" >&2
    exit 127
}
command -v identify >/dev/null 2>&1 || {
    echo "error: ImageMagick 'identify' is required to validate the PNG" >&2
    exit 127
}

mkdir -p "$OUT/cursor-theme/minde-test/cursors"
guile --no-auto-compile tests/fixtures/cursor-theme.scm \
    "$OUT/cursor-theme/minde-test/cursors/default"
export XCURSOR_PATH="$OUT/cursor-theme" XCURSOR_THEME=minde-test XCURSOR_SIZE=24

nested_start "$OUT" "${MINDE_SCREENCAPTURE_E2E_DISPLAY:-:96}" || {
    echo "error: nested compositor failed; inspect $OUT" >&2
    exit 1
}

nested_wayland timeout 20 "$NESTED_PROTOCOL_CLIENT" capture-clock >"$OUT/capture-clock.log" 2>&1 || {
    cat "$OUT/capture-clock.log" >&2
    exit 1
}
cat "$OUT/capture-clock.log"

for scenario in scene-layers scene-message scene-cursor; do
    timeout 45 "$NESTED_PROTOCOL_CLIENT" "$scenario" >"$OUT/$scenario.log" 2>&1 || {
        cat "$OUT/$scenario.log" >&2
        exit 1
    }
    cat "$OUT/$scenario.log"
done

nested_wayland env MINDE_CAPTURE_HOST_DISPLAY="$DISPLAY" timeout 60 \
    "$NESTED_PROTOCOL_CLIENT" capture-lifecycle >"$OUT/capture-lifecycle.log" 2>&1 || {
    cat "$OUT/capture-lifecycle.log" >&2
    exit 1
}
cat "$OUT/capture-lifecycle.log"

# (wm-outputs) -> ((id x y w h name) ...); ask the running compositor for
# the real mode instead of assuming the Xvfb geometry, so this gate cannot
# silently pass against the wrong dimensions if that ever changes.
outputs=$(nested_wayland scripts/minde-cmd '(wm-outputs)' 2>/dev/null) \
    || { echo "error: wm-outputs query failed" >&2; exit 1; }
expect_w=$(printf '%s' "$outputs" | sed -n 's/.*(0 [0-9-]* [0-9-]* \([0-9]*\) .*/\1/p')
expect_h=$(printf '%s' "$outputs" | sed -n 's/.*(0 [0-9-]* [0-9-]* [0-9]* \([0-9]*\).*/\1/p')
if [ -z "${expect_w:-}" ] || [ -z "${expect_h:-}" ]; then
    echo "error: could not parse output geometry from: $outputs" >&2
    exit 1
fi

png="$OUT/grim-capture.png"
nested_wayland timeout 20 grim "$png" >"$OUT/grim.log" 2>&1 || {
    echo "error: grim exited non-zero capturing ext-image-copy-capture-v1" >&2
    cat "$OUT/grim.log" >&2 || true
    exit 1
}

[ -s "$png" ] || { echo "error: grim produced an empty file" >&2; exit 1; }

identify_out=$(identify -format '%m %w %h %[standard-deviation]' "$png" 2>"$OUT/identify.log") || {
    echo "error: captured file is not a parseable image" >&2
    cat "$OUT/identify.log" >&2 || true
    exit 1
}
fmt=$(printf '%s' "$identify_out" | awk '{print $1}')
w=$(printf '%s' "$identify_out" | awk '{print $2}')
h=$(printf '%s' "$identify_out" | awk '{print $3}')
stddev=$(printf '%s' "$identify_out" | awk '{print $4}')

[ "$fmt" = "PNG" ] || { echo "error: grim did not produce a PNG (got $fmt)" >&2; exit 1; }
if [ "$w" != "$expect_w" ] || [ "$h" != "$expect_h" ]; then
    echo "error: captured image is ${w}x${h}, expected ${expect_w}x${expect_h} (wm-outputs)" >&2
    exit 1
fi

# A capture bug that ships a blank/uninitialized buffer typically renders as
# a single flat color (stddev == 0). The nested compositor's own chrome
# (background plus any decoration) makes a truly flat frame unlikely if the
# capture path is actually reading composited output.
stddev_zero=$(awk -v s="$stddev" 'BEGIN { print (s == 0 || s == "0") ? "1" : "0" }')
if [ "$stddev_zero" = "1" ]; then
    echo "error: captured image is a single flat color (stddev=$stddev); capture buffer looks blank" >&2
    exit 1
fi

echo "ok - grim (ext-image-copy-capture-v1) captured a valid ${w}x${h} PNG (stddev=$stddev)"

# --- wlr-screencopy-unstable-v1 via wf-recorder -----------------------------
# wf-recorder speaks only wlr-screencopy (not ext-image-copy-capture), so it
# exercises the hand-written protocol path. Record for a couple of seconds,
# stop with SIGINT (wf-recorder finalizes the muxer on INT), then validate the
# file non-interactively. This is the protocol xdg-desktop-portal-wlr uses.
if ! command -v wf-recorder >/dev/null 2>&1; then
    echo "warn - wf-recorder not found; skipping wlr-screencopy check" >&2
else
    vid="$OUT/wf-recorder-capture.mkv"
    rm -f "$vid"
    # Bounded: launch in background, let it run a few seconds, then SIGINT and
    # reap. `exec` in the subshell makes wf-recorder itself the job PID (not a
    # wrapper shell), so the SIGINT that finalizes the muxer reaches it -- see
    # nested_wayland for the client environment being replicated here.
    (
        exec env DISPLAY= WAYLAND_DISPLAY="$NESTED_WAYLAND_DISPLAY" \
            XDG_RUNTIME_DIR="$NESTED_RT" HOME="$NESTED_RT/home" \
            XDG_CACHE_HOME="$NESTED_RT/cache" XDG_CONFIG_HOME="$NESTED_RT/config" \
            XDG_STATE_HOME="$NESTED_RT/state" \
            wf-recorder -f "$vid"
    ) >"$OUT/wf-recorder.log" 2>&1 &
    wfr_pid=$!
    sleep 3
    if ! kill -0 "$wfr_pid" 2>/dev/null; then
        echo "error: wf-recorder exited early capturing wlr-screencopy" >&2
        cat "$OUT/wf-recorder.log" >&2 || true
        exit 1
    fi
    kill -INT "$wfr_pid" 2>/dev/null || true
    # Give it a bounded window to flush and finalize the container.
    reaped=0
    for _ in 1 2 3 4 5 6 7 8 9 10; do
        if ! kill -0 "$wfr_pid" 2>/dev/null; then
            reaped=1
            break
        fi
        sleep 1
    done
    if [ "$reaped" != "1" ]; then
        kill -KILL "$wfr_pid" 2>/dev/null || true
        echo "error: wf-recorder did not stop after SIGINT" >&2
        cat "$OUT/wf-recorder.log" >&2 || true
        exit 1
    fi
    wait "$wfr_pid" 2>/dev/null || true

    [ -s "$vid" ] || {
        echo "error: wf-recorder produced an empty file" >&2
        cat "$OUT/wf-recorder.log" >&2 || true
        exit 1
    }

    # If ffprobe (ffmpeg) is available, confirm the file is a parseable video
    # with a real pixel geometry; otherwise a non-empty file is the floor.
    if command -v ffprobe >/dev/null 2>&1; then
        dims=$(ffprobe -v error -select_streams v:0 \
            -show_entries stream=width,height -of csv=s=x:p=0 \
            "$vid" 2>"$OUT/ffprobe.log") || {
            echo "error: ffprobe could not parse the wf-recorder output" >&2
            cat "$OUT/ffprobe.log" >&2 || true
            exit 1
        }
        vw=${dims%x*}
        vh=${dims#*x}
        if [ -z "$vw" ] || [ -z "$vh" ] || [ "$vw" -le 0 ] || [ "$vh" -le 0 ]; then
            echo "error: wf-recorder video has no valid geometry (got '$dims')" >&2
            exit 1
        fi
        echo "ok - wf-recorder (wlr-screencopy) recorded a ${vw}x${vh} video"
    else
        echo "ok - wf-recorder (wlr-screencopy) recorded a non-empty video (ffprobe absent)"
    fi
fi

echo "screencapture-e2e: all checks passed"
