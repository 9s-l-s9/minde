#!/bin/sh
# SPDX-License-Identifier: GPL-3.0-or-later
# Nested application compatibility matrix.  Results and failure artifacts are
# retained under MINDE_APPS_OUT (default /tmp/minde-applications).
set -eu
cd "$(dirname "$0")/.."
. tests/lib/nested-compositor.sh

OUT=${MINDE_APPS_OUT:-/tmp/minde-applications}
RESULTS="$OUT/results.jsonl"
SUMMARY="$OUT/summary.txt"
STRICT=${MINDE_APPS_STRICT:-0}
FILTER=${MINDE_APPS_FILTER:-all}
FAILURES=0
PASSES=0
SKIPS=0
mkdir -p "$OUT"
: >"$RESULTS"
: >"$SUMMARY"

cleanup() {
    if [ -n "${NESTED_WAYLAND_DISPLAY:-}" ]; then
        nested_wayland eww --config "$PWD/doc/eww" kill >/dev/null 2>&1 || true
    fi
    nested_stop
}
trap cleanup EXIT HUP INT TERM

record() {
    scenario=$1 toolkit=$2 protocol=$3 requirement=$4 status=$5 duration=$6 artifact=$7 reason=$8
    jq -cn \
        --arg scenario "$scenario" --arg toolkit "$toolkit" \
        --arg protocol "$protocol" --arg requirement "$requirement" \
        --arg status "$status" --arg artifact "$artifact" --arg reason "$reason" \
        --argjson duration_ms "$duration" \
        '{schema_version:1,scenario:$scenario,toolkit:$toolkit,protocol:$protocol,
          requirement:$requirement,status:$status,duration_ms:$duration_ms,
          artifact:(if ($artifact|length)>0 then $artifact else null end),
          reason:(if ($reason|length)>0 then $reason else null end)}' \
        >>"$RESULTS"
    printf '%-20s %-8s %s\n' "$scenario" "$status" "$reason" | tee -a "$SUMMARY"
}

selected() {
    [ "$FILTER" = all ] && return 0
    case ",$FILTER," in
        *",$1,"*) return 0 ;;
        *) return 1 ;;
    esac
}

scenario_start() {
    env DISPLAY= WAYLAND_DISPLAY="$NESTED_WAYLAND_DISPLAY" \
        XDG_RUNTIME_DIR="$NESTED_RT" HOME="$NESTED_RT/home" \
        XDG_CACHE_HOME="$NESTED_RT/cache" XDG_CONFIG_HOME="$NESTED_RT/config" \
        XDG_STATE_HOME="$NESTED_RT/state" \
        setsid timeout -k 2 20 "$@" &
    SCENARIO_PID=$!
}

scenario_stop() {
    pid=$1
    kill -TERM "-$pid" 2>/dev/null || kill "$pid" 2>/dev/null || true
    sleep 1
    kill -KILL "-$pid" 2>/dev/null || true
    wait "$pid" 2>/dev/null || true
}

skip_or_fail() {
    scenario=$1 toolkit=$2 protocol=$3 requirement=$4 reason=$5
    if [ "$requirement" = required ] || [ "$STRICT" = 1 ]; then
        FAILURES=$((FAILURES + 1))
        record "$scenario" "$toolkit" "$protocol" "$requirement" fail 0 "" "$reason"
    else
        SKIPS=$((SKIPS + 1))
        record "$scenario" "$toolkit" "$protocol" "$requirement" skip 0 "" "$reason"
    fi
}

run_toplevel() {
    scenario=$1 toolkit=$2 protocol=$3 requirement=$4 executable=$5
    shift 5
    selected "$scenario" || return 0
    if ! command -v "$executable" >/dev/null 2>&1; then
        skip_or_fail "$scenario" "$toolkit" "$protocol" "$requirement" "missing executable: $executable"
        return
    fi
    line=$(($(wc -l <"$NESTED_LOG") + 1))
    started=$(date +%s%3N)
    output="$OUT/$scenario.log"
    scenario_start "$@" >"$output" 2>&1
    pid=$SCENARIO_PID
    if nested_wait_for_log_after "$line" \
        'protocol="wayland".*managed toplevel mapped\|managed toplevel mapped.*protocol="wayland"' 12; then
        artifact="$OUT/$scenario.png"
        nested_capture "$artifact"
        duration=$(($(date +%s%3N) - started))
        PASSES=$((PASSES + 1))
        record "$scenario" "$toolkit" "$protocol" "$requirement" pass "$duration" "$artifact" ""
    else
        artifact="$OUT/$scenario-failure.png"
        nested_capture "$artifact"
        duration=$(($(date +%s%3N) - started))
        FAILURES=$((FAILURES + 1))
        record "$scenario" "$toolkit" "$protocol" "$requirement" fail "$duration" "$artifact" "no managed window appeared"
    fi
    scenario_stop "$pid"
    sleep 1
}

run_xterm() {
    selected xterm || return 0
    if ! command -v xterm >/dev/null 2>&1; then
        skip_or_fail xterm Xlib xwayland required "missing executable: xterm"
        return
    fi
    line=$(($(wc -l <"$NESTED_LOG") + 1))
    started=$(date +%s%3N)
    scripts/mindectl eval '(wm-spawn "timeout 20 xterm -fa monospace -title minde-test-xterm")' >/dev/null
    if nested_wait_for_log_after "$line" \
        'protocol="xwayland".*managed toplevel mapped\|managed toplevel mapped.*protocol="xwayland"' 12; then
        artifact="$OUT/xterm.png"
        nested_capture "$artifact"
        PASSES=$((PASSES + 1))
        record xterm Xlib xwayland required pass "$(($(date +%s%3N) - started))" "$artifact" ""
    else
        FAILURES=$((FAILURES + 1))
        record xterm Xlib xwayland required fail "$(($(date +%s%3N) - started))" "" "no Xwayland window appeared"
    fi
}

run_clipboard() {
    selected wl-clipboard || return 0
    if ! command -v wl-copy >/dev/null 2>&1 || ! command -v wl-paste >/dev/null 2>&1; then
        skip_or_fail wl-clipboard wlroots wayland required "wl-copy and wl-paste are required"
        return
    fi
    started=$(date +%s%3N)
    nested_wayland wl-copy --type text/plain minde-sprint-7 >"$OUT/wl-clipboard.log" 2>&1 &
    copy_pid=$!
    sleep 1
    actual=$(nested_wayland timeout 5 wl-paste --no-newline 2>>"$OUT/wl-clipboard.log" || true)
    kill "$copy_pid" 2>/dev/null || true
    wait "$copy_pid" 2>/dev/null || true
    if [ "$actual" = minde-sprint-7 ]; then
        PASSES=$((PASSES + 1))
        record wl-clipboard wlroots wayland required pass "$(($(date +%s%3N) - started))" "$OUT/wl-clipboard.log" ""
    else
        FAILURES=$((FAILURES + 1))
        record wl-clipboard wlroots wayland required fail "$(($(date +%s%3N) - started))" "$OUT/wl-clipboard.log" "clipboard round trip failed"
    fi
}

run_qt() {
    scenario=$1 major=$2
    selected "$scenario" || return 0
    if ! command -v qmlscene >/dev/null 2>&1; then
        skip_or_fail "$scenario" "Qt$major" wayland optional "missing executable: qmlscene"
        return
    fi
    if ! ldd "$(command -v qmlscene)" 2>/dev/null | grep -q "libQt${major}Core"; then
        skip_or_fail "$scenario" "Qt$major" wayland optional "qmlscene is not linked to Qt $major"
        return
    fi
    run_toplevel "$scenario" "Qt$major" wayland optional qmlscene \
        env QT_QPA_PLATFORM=wayland qmlscene "$PWD/tests/clients/window.qml"
}

run_layer() {
    scenario=$1 toolkit=$2 requirement=$3 executable=$4 namespace=$5
    shift 5
    selected "$scenario" || return 0
    if ! command -v "$executable" >/dev/null 2>&1; then
        skip_or_fail "$scenario" "$toolkit" layer-shell "$requirement" "missing executable: $executable"
        return
    fi
    line=$(($(wc -l <"$NESTED_LOG") + 1))
    started=$(date +%s%3N)
    scenario_start "$@" >"$OUT/$scenario.log" 2>&1
    pid=$SCENARIO_PID
    if nested_wait_for_log_after "$line" "layer surface mapped.*$namespace\|namespace.*$namespace.*layer surface mapped" 12; then
        artifact="$OUT/$scenario.png"
        nested_capture "$artifact"
        PASSES=$((PASSES + 1))
        record "$scenario" "$toolkit" layer-shell "$requirement" pass "$(($(date +%s%3N) - started))" "$artifact" ""
    else
        artifact="$OUT/$scenario-failure.png"
        nested_capture "$artifact"
        FAILURES=$((FAILURES + 1))
        record "$scenario" "$toolkit" layer-shell "$requirement" fail "$(($(date +%s%3N) - started))" "$artifact" "layer namespace was not mapped"
    fi
    scenario_stop "$pid"
    sleep 1
}

run_swaylock() {
    selected swaylock || return 0
    if ! command -v swaylock >/dev/null 2>&1; then
        skip_or_fail swaylock swaylock session-lock manual "missing executable: swaylock"
        return
    fi
    line=$(($(wc -l <"$NESTED_LOG") + 1))
    started=$(date +%s%3N)
    scenario_start swaylock --color 282828 >"$OUT/swaylock.log" 2>&1
    pid=$SCENARIO_PID
    if nested_wait_for_log_after "$line" 'session lock surface mapped' 5; then
        artifact="$OUT/swaylock.png"
        nested_capture "$artifact"
        PASSES=$((PASSES + 1))
        record swaylock swaylock session-lock manual pass \
            "$(($(date +%s%3N) - started))" "$artifact" ""
    elif grep -q 'Missing ext-session-lock-v1' "$OUT/swaylock.log"; then
        if [ "$STRICT" = 1 ]; then
            FAILURES=$((FAILURES + 1))
            record swaylock swaylock session-lock manual fail \
                "$(($(date +%s%3N) - started))" "$OUT/swaylock.log" \
                "ext-session-lock-v1 is not implemented"
        else
            SKIPS=$((SKIPS + 1))
            record swaylock swaylock session-lock manual skip \
                "$(($(date +%s%3N) - started))" "$OUT/swaylock.log" \
                "ext-session-lock-v1 is not implemented"
        fi
    else
        FAILURES=$((FAILURES + 1))
        record swaylock swaylock session-lock manual fail \
            "$(($(date +%s%3N) - started))" "$OUT/swaylock.log" \
            "lock surface was not mapped"
    fi
    scenario_stop "$pid"
}

if ! nested_start "$OUT" "${MINDE_APPS_DISPLAY:-:96}"; then
    echo "error: nested compositor failed; inspect $OUT" >&2
    exit 1
fi
command -v setsid >/dev/null 2>&1 || {
    echo "error: setsid is required for bounded application process groups" >&2
    exit 127
}

run_toplevel foot terminal wayland required foot \
    foot --app-id=minde-test-foot --title=minde-test-foot sh -c 'sleep 15'
run_clipboard
run_toplevel gtk3 GTK3 wayland optional gtk3-demo \
    env GDK_BACKEND=wayland gtk3-demo
run_toplevel gtk4 GTK4 wayland optional gtk4-demo \
    env GDK_BACKEND=wayland gtk4-demo
run_qt qt5 5
run_qt qt6 6
run_toplevel electron Electron wayland optional electron \
    electron --ozone-platform=wayland --no-sandbox "$PWD/tests/clients/electron.html"
run_toplevel chromium Chromium wayland optional chromium \
    chromium --ozone-platform=wayland --no-sandbox \
    --user-data-dir="$NESTED_RT/chromium-profile" --disk-cache-size=1048576 \
    --media-cache-size=1048576 about:blank
mkdir -p "$NESTED_RT/firefox-profile"
run_toplevel firefox Firefox wayland optional firefox \
    env MOZ_ENABLE_WAYLAND=1 firefox --no-remote --profile "$NESTED_RT/firefox-profile" about:blank
run_toplevel emacs-pgtk Emacs wayland optional emacs \
    env GDK_BACKEND=wayland emacs -Q --name minde-test-emacs \
    --eval '(progn (switch-to-buffer "minde-test") (insert "Emacs PGTK / Wayland") (run-at-time 15 nil (quote kill-emacs)))'
run_toplevel sdl2 SDL2 wayland optional love \
    env SDL_VIDEODRIVER=wayland love "$PWD/tests/clients"
run_xterm

if command -v convert >/dev/null 2>&1; then
    convert -size 32x32 xc:'#282828' "$OUT/background.png"
fi
run_layer swaybg swaybg optional swaybg wallpaper \
    swaybg -m fill -i "$OUT/background.png"
run_layer fuzzel fuzzel optional fuzzel launcher \
    sh -c "printf 'one\ntwo\n' | fuzzel --dmenu"
run_swaylock
run_layer eww eww optional eww gtk-layer-shell \
    sh -c "eww --config '$PWD/doc/eww' daemon; sleep 2; eww --config '$PWD/doc/eww' open bar; sleep 15"
nested_wayland eww --config "$PWD/doc/eww" kill >/dev/null 2>&1 || true

jq -s '{schema_version:1,total:length,pass:map(select(.status=="pass"))|length,
       skip:map(select(.status=="skip"))|length,fail:map(select(.status=="fail"))|length,
       scenarios:.}' "$RESULTS" >"$OUT/results.json"
printf 'application matrix: %s passed, %s skipped, %s failed\n' "$PASSES" "$SKIPS" "$FAILURES"
[ "$FAILURES" -eq 0 ]
