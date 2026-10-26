#!/bin/sh
# SPDX-License-Identifier: GPL-3.0-or-later
# Exercise actual confinement/lock events, EWMH and xdg fullscreen, resize
# policy and fullscreen panel rendering/input in an isolated compositor.
set -eu
cd "$(dirname "$0")/.."
. tests/lib/nested-compositor.sh
OUT=${MINDE_FULLSCREEN_POINTER_E2E_OUT:-/tmp/minde-fullscreen-pointer-e2e}
CLIENT_PID=
PANEL_PID=
SECOND_PID=
cleanup() {
    for pid in "$CLIENT_PID" "$PANEL_PID" "$SECOND_PID"; do
        [ -z "$pid" ] || kill "$pid" 2>/dev/null || true
    done
    nested_stop
}
trap cleanup EXIT HUP INT TERM
mkdir -p "$OUT"
protocols=$(pkg-config --variable=pkgdatadir wayland-protocols)
# Use the protocol already pinned by Cargo, including vendored builds.
wlr_manifest=$(cargo metadata --offline --locked --format-version 1 | jq -r '.packages[] | select(.name == "wayland-protocols-wlr") | .manifest_path')
wlr_protocols=$(dirname "$wlr_manifest")/wlr-protocols
for spec in "xdg-shell $protocols/stable/xdg-shell/xdg-shell.xml" \
    "pointer-constraints $protocols/unstable/pointer-constraints/pointer-constraints-unstable-v1.xml" \
    "relative-pointer $protocols/unstable/relative-pointer/relative-pointer-unstable-v1.xml" \
    "wlr-layer-shell $wlr_protocols/unstable/wlr-layer-shell-unstable-v1.xml"; do
    name=${spec%% *}
    xml=${spec#* }
    wayland-scanner client-header "$xml" "$OUT/$name-client-protocol.h"
    wayland-scanner private-code "$xml" "$OUT/$name-protocol.c"
done
# pkg-config flags intentionally word-split.
# shellcheck disable=SC2046
gcc -Wall -Wextra -Werror -I"$OUT" tests/clients/pointer-constraints.c \
    "$OUT/xdg-shell-protocol.c" "$OUT/pointer-constraints-protocol.c" \
    "$OUT/relative-pointer-protocol.c" "$OUT/wlr-layer-shell-protocol.c" \
    $(pkg-config --cflags --libs wayland-client) -o "$OUT/pointer-client"
# shellcheck disable=SC2046
gcc -Wall -Wextra -Werror tests/clients/x11-fullscreen.c \
    $(pkg-config --cflags --libs x11) -o "$OUT/x11-client"

nested_start "$OUT" "${MINDE_FULLSCREEN_POINTER_E2E_DISPLAY:-:91}"
eval_wm() { timeout 5 scripts/mindectl eval "$1"; }
fail() { echo "FAIL: $* (logs in $OUT)" >&2; exit 1; }
wait_log() {
    attempt=0
    until grep -q "$2" "$1"; do
        attempt=$((attempt + 1)); [ "$attempt" -lt 80 ] || fail "missing $2 in $1"
        sleep 0.1
    done
}
check_expr() {
    attempt=0
    while [ "$(eval_wm "$2")" != '#t' ]; do
        attempt=$((attempt + 1)); [ "$attempt" -lt 50 ] || fail "$1"
        sleep 0.1
    done
    echo "ok - $1"
}
command_client() {
    previous=$(grep -c "^done $1$" "$CLIENT_LOG" || true)
    printf '%s\n' "$1" >&3
    attempt=0
    while [ "$(grep -c "^done $1$" "$CLIENT_LOG" || true)" -le "$previous" ]; do
        attempt=$((attempt + 1)); [ "$attempt" -lt 80 ] || fail "command $1 did not complete"
        sleep 0.1
    done
}
stop_client() {
    printf 'quit\n' >&3
    wait "$CLIENT_PID"
    CLIENT_PID=
    exec 3>&-
    check_expr 'client unmapped' '(null? (all-window-ids))'
}
start_fifo() {
    rm -f "$OUT/commands"
    mkfifo "$OUT/commands"
    exec 3<>"$OUT/commands"
}
start_fifo
CLIENT_LOG="$OUT/pointer.log"
nested_wayland timeout 120 "$OUT/pointer-client" <"$OUT/commands" >"$CLIENT_LOG" 2>&1 &
CLIENT_PID=$!
nested_wait_for_window_after 0 15
wait_log "$CLIENT_LOG" 'size '
eval_wm '(begin (split-frame-horizontal!) (wm-warp-pointer 50 50))' >/dev/null
wait_log "$CLIENT_LOG" 'enter '
command_client confine-null
wait_log "$CLIENT_LOG" '^confined$'
nested_wayland timeout 5 wlrctl pointer move 800 0
check_expr 'null-region confinement stays inside the client' '(let* ((r (wm-window-geometry (focused-window-id))) (p (wm-pointer-position))) (and (> (car p) (- (+ (car r) (caddr r)) 2)) (<= (car p) (+ (car r) (caddr r)))))'
! grep -q '^unconfined$' "$CLIENT_LOG" || fail 'confinement escaped'
command_client release
eval_wm '(wm-warp-pointer 50 50)' >/dev/null
command_client confine-hole
nested_wayland timeout 5 wlrctl pointer move 100 0
check_expr 'motion cannot cross a subtracted region' '(let ((x (car (wm-pointer-position)))) (and (> x 81) (<= x 83)))'
command_client release
eval_wm '(wm-warp-pointer 50 50)' >/dev/null
command_client confine-input
nested_wayland timeout 5 wlrctl pointer move 800 0
check_expr 'null constraint respects a smaller input region' '(let ((x (car (wm-pointer-position)))) (and (> x 201) (<= x 203)))'
command_client release
eval_wm '(wm-warp-pointer 50 50)' >/dev/null
command_client lock
wait_log "$CLIENT_LOG" '^locked$'
nested_wayland timeout 5 wlrctl pointer move 30 40
wait_log "$CLIENT_LOG" 'relative 30.000 40.000'
check_expr 'locked pointer stays parked while relative motion flows' '(equal? (wm-pointer-position) (list 50 50))'
command_client hint
command_client release
check_expr 'unlock honors the committed cursor hint' '(equal? (wm-pointer-position) (list 53 63))'
# Reproduce #2 through actual host motion. These commands address Xvfb,
# never the user's desktop or the nested Xwayland server.
DISPLAY="$NESTED_DISPLAY" timeout 5 xdotool mousemove --sync 50 50
check_expr 'host absolute sample positions the pointer' '(equal? (wm-pointer-position) (list 50 50))'
command_client lock
relative_count() { grep -c '^relative ' "$CLIENT_LOG" || true; }
assert_host_delta() {
    x=$1 expected=$2
    before=$(relative_count)
    DISPLAY="$NESTED_DISPLAY" timeout 5 xdotool mousemove --sync "$x" 50
    attempt=0
    while [ "$(relative_count)" -le "$before" ]; do
        attempt=$((attempt + 1)); [ "$attempt" -lt 50 ] || fail 'host motion did not reach relative pointer'
        sleep 0.1
    done
    actual=$(grep '^relative ' "$CLIENT_LOG" | tail -1)
    [ "$actual" = "relative $expected 0.000" ] || fail "absolute motion: $actual != $expected"
}
assert_host_delta 60 10.000
assert_host_delta 70 10.000
check_expr 'successive absolute lock motions keep the pointer parked' '(equal? (wm-pointer-position) (list 50 50))'
echo 'ok - two 10px host motions produce two 10px relative deltas'
eval_wm '(wm-warp-pointer 55 50)' >/dev/null
check_expr 'compositor warp establishes a new cursor position' '(equal? (wm-pointer-position) (list 55 50))'
assert_host_delta 80 0.000
assert_host_delta 90 10.000
echo 'ok - absolute relative deltas restart without a jump after a warp'
command_client release
stop_client

eval_wm '(begin (collapse-to-one-frame!) (wm-border-color "#00ff00"))' >/dev/null
# Keep stdin open without a pipe subprocess; only this isolated panel uses fd 4.
rm -f "$OUT/panel-commands"
mkfifo "$OUT/panel-commands"
exec 4<>"$OUT/panel-commands"
nested_wayland timeout 120 "$OUT/pointer-client" panel <"$OUT/panel-commands" >"$OUT/panel.log" 2>&1 &
PANEL_PID=$!
wait_log "$OUT/panel.log" 'panel-ready'
assert_pixel() {
    label=$1 x=$2 y=$3 expected=$4
    attempt=0
    while :; do
        nested_wayland timeout 10 grim "$OUT/$label.png"
        actual=$(convert "$OUT/$label.png" -format "%[hex:p{$x,$y}]" info:)
        [ "$actual" != "$expected" ] || break
        attempt=$((attempt + 1)); [ "$attempt" -lt 30 ] || fail "$label: $actual != $expected"
        sleep 0.1
    done
    echo "ok - $label"
}
# Both native and X11 clients must fill the output, cover the panel/border,
# receive pointer input at the top edge, and restore tiling on exit.
xdisplay=":$(eval_wm '(caddr (wm-runtime-info))')"
for kind in native x11 initial-x11; do
    start_fifo
    CLIENT_LOG="$OUT/$kind.log"
    if [ "$kind" = native ]; then
        nested_wayland timeout 90 "$OUT/pointer-client" <"$OUT/commands" >"$CLIENT_LOG" 2>&1 &
    elif [ "$kind" = x11 ]; then
        env DISPLAY="$xdisplay" timeout 90 "$OUT/x11-client" <"$OUT/commands" >"$CLIENT_LOG" 2>&1 &
    else
        env DISPLAY="$xdisplay" timeout 90 "$OUT/x11-client" fullscreen <"$OUT/commands" >"$CLIENT_LOG" 2>&1 &
    fi
    CLIENT_PID=$!
    nested_wait_for_window_after 0 15
    if [ "$kind" != initial-x11 ]; then
        assert_pixel "$kind-panel-visible" 10 10 A02020
        if [ "$kind" = x11 ]; then
            command_client resize
            # Inspect real X11 geometry, not the compositor's cached rectangle.
            sleep 0.3
            actual=$(env DISPLAY="$xdisplay" xdotool search --name '^minde-fullscreen-test$' getwindowgeometry --shell)
            printf '%s\n' "$actual" | grep -q '^WIDTH=1274$' || fail 'tiled X11 self-resize changed width'
            echo 'ok - X11 client self-resize cannot shrink the tiled frame'
            eval_wm '(wm-warp-pointer 50 50)' >/dev/null
            command_client grab
            wait_log "$CLIENT_LOG" '^grab 0$'
            nested_wayland timeout 5 wlrctl pointer move -100 -100
            check_expr 'Xwayland grab confines the pointer to its window' '(let ((p (wm-pointer-position))) (and (>= (car p) 3) (>= (cadr p) 37)))'
            command_client ungrab
        fi
        command_client fullscreen
    fi
    check_expr "$kind fullscreen geometry" '(and (fullscreen-window) (equal? (wm-window-geometry (focused-window-id)) (list 0 0 1280 800)))'
    assert_pixel "$kind-covers-panel" 10 10 204060
    assert_pixel "$kind-hides-frame-border" 1 100 204060
    if [ "$kind" = native ]; then
        eval_wm '(wm-warp-pointer 10 10)' >/dev/null
        wait_log "$CLIENT_LOG" 'enter 10.000 10.000'
        echo 'ok - fullscreen receives input over the hidden panel'
    fi
# Reproduce #1 with actual protocol state, size and keyboard events. A second
# group must work while the first group's fullscreen client is parked.
if [ "$kind" = native ]; then
    original_group=$(eval_wm '(current-group-name)')
    original_id=$(eval_wm '(focused-window-id)')
    eval_wm '(begin (create-group! "fullscreen-regression") #t)' >/dev/null
    check_expr 'empty group has no inherited fullscreen flag' '(and (not (fullscreen-window)) (not (focused-window-id)))'
    check_expr 'old fullscreen client is parked' "(not (wm-window-geometry $original_id))"
    rm -f "$OUT/second-commands"
    mkfifo "$OUT/second-commands"
    exec 5<>"$OUT/second-commands"
    nested_wayland timeout 60 "$OUT/pointer-client" <"$OUT/second-commands" >"$OUT/group-second.log" 2>&1 &
    SECOND_PID=$!
    nested_wait_for_window_after 0 15
    second_id=$(eval_wm '(focused-window-id)')
    check_expr 'second group places its new client in the frame' '(equal? (wm-window-geometry (focused-window-id)) (list 3 37 1274 760))'
    wait_log "$OUT/group-second.log" '^size 1274 760$'
    eval_wm '(wm-send-key 0 "a")' >/dev/null
    wait_log "$OUT/group-second.log" '^key 30 1$'
    echo 'ok - keyboard input reaches the second group'
    # Give both groups independent fullscreen state.
    eval_wm '(fullscreen!)' >/dev/null
    check_expr 'second group enters its own fullscreen' "(equal? (fullscreen-window) $second_id)"
    eval_wm "(begin (switch-to-group! $original_group) (wm-clear-message) #t)" >/dev/null
    check_expr 'returning restores original fullscreen and focus' "(and (equal? (fullscreen-window) $original_id) (equal? (focused-window-id) $original_id) (equal? (wm-window-geometry $original_id) (list 0 0 1280 800)))"
    wait_log "$CLIENT_LOG" '^keyboard-enter$'
    eval_wm '(wm-send-key 0 "b")' >/dev/null
    wait_log "$CLIENT_LOG" '^key 48 1$'
    echo 'ok - keyboard input returns to the original fullscreen client'
    assert_pixel 'returned-fullscreen-covers-panel' 10 10 204060
    # Closing the other group's hidden fullscreen client must not clear ours.
    printf 'quit\n' >&5
    wait "$SECOND_PID"
    SECOND_PID=
    exec 5>&-
    check_expr 'hidden fullscreen close preserves current fullscreen' "(equal? (fullscreen-window) $original_id)"
    eval_wm '(begin (switch-to-group! "fullscreen-regression") #t)' >/dev/null
    check_expr 'closed hidden fullscreen leaves a usable empty group' '(and (not (fullscreen-window)) (not (focused-window-id)))'
    eval_wm "(begin (switch-to-group! $original_group) (wm-clear-message) #t)" >/dev/null
    check_expr 'original fullscreen survives another round trip' "(equal? (fullscreen-window) $original_id)"
fi
    # Output geometry changes must reach fullscreen clients too (#3).
    nested_wayland timeout 10 wlr-randr --output winit --scale 2
    check_expr "$kind fullscreen follows output scale" '(equal? (wm-window-geometry (focused-window-id)) (list 0 0 640 400))'
    if [ "$kind" = native ]; then
        wait_log "$CLIENT_LOG" '^size 640 400$'
    else
        actual=$(env DISPLAY="$xdisplay" xdotool search --name '^minde-fullscreen-test$' getwindowgeometry --shell)
        printf '%s\n' "$actual" | grep -q '^WIDTH=640$' || fail 'X11 fullscreen did not resize with output'
    fi
    nested_wayland timeout 10 wlr-randr --output winit --scale 1
    check_expr "$kind fullscreen follows restored output scale" '(equal? (wm-window-geometry (focused-window-id)) (list 0 0 1280 800))'
    command_client unfullscreen
    check_expr "$kind returns to tiled geometry" '(and (not (fullscreen-window)) (equal? (wm-window-geometry (focused-window-id)) (list 3 37 1274 760)))'
    assert_pixel "$kind-panel-restored" 10 10 A02020
    stop_client
done
! nested_log_has 'protocol error' || fail 'protocol error'
echo 'fullscreen-pointer-e2e: all checks passed'
