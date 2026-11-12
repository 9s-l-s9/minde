#!/bin/sh
# SPDX-License-Identifier: GPL-3.0-or-later
# Nested (winit) end-to-end gate for the keyboard layout group surface:
# minde is started with two XKB groups (de bone + us) and the Scheme
# wrappers `keyboard-layouts' / `set-keyboard-layout!' are driven with
# `mindectl eval', checking that the active flag follows the switch and
# that `handle-keyboard-layout-changed!' fires with the group name.
set -eu

cd "$(dirname "$0")/.."
. tests/lib/nested-compositor.sh

OUT=${MINDE_KEYBOARD_LAYOUT_E2E_OUT:-/tmp/swm-kblayout-e2e}
export NESTED_XKB_LAYOUT=de,us NESTED_XKB_VARIANT=bone,
trap nested_stop EXIT HUP INT TERM

nested_start "$OUT" "${MINDE_KEYBOARD_LAYOUT_E2E_DISPLAY:-:99}" || {
    echo "error: nested compositor failed; inspect $OUT" >&2
    exit 1
}

fail() {
    echo "error: $1" >&2
    shift
    for f in "$@"; do cat "$f" >&2 2>/dev/null || true; done
    exit 1
}

seval() {
    timeout 15 scripts/mindectl eval "$1"
}

active() {
    seval '(assq-ref (find (lambda (l) (assq-ref l (quote active))) (keyboard-layouts)) (quote name))'
}

count=$(seval '(length (keyboard-layouts))') || fail "eval (keyboard-layouts) failed"
[ "$count" = "2" ] || fail "expected two layout groups, got $count"
[ "$(active)" = '"German (Bone)"' ] || fail "initial group is not German (Bone): $(active)"
echo "ok - (keyboard-layouts) lists both groups, de(bone) active"

seval '(begin (define %kb-changes (quote ()))
              (define (handle-keyboard-layout-changed! name)
                (set! %kb-changes (cons name %kb-changes)))
              #t)' >/dev/null

r=$(seval '(set-keyboard-layout! (quote next))') || fail "set-keyboard-layout! eval failed"
[ "$r" = "#t" ] || fail "set-keyboard-layout! next did not return #t: $r"
sleep 0.5
[ "$(active)" = '"English (US)"' ] || fail "next did not activate English (US): $(active)"
echo "ok - (set-keyboard-layout! 'next) activates English (US)"

r=$(seval '(set-keyboard-layout! "german")')
sleep 0.5
[ "$(active)" = '"German (Bone)"' ] || fail "name prefix did not activate German (Bone): $(active)"
echo "ok - (set-keyboard-layout! \"german\") switches back by name prefix"

changes=$(seval '%kb-changes')
[ "$changes" = '("German (Bone)" "English (US)")' ] ||
    fail "handle-keyboard-layout-changed! did not see both switches: $changes"
echo "ok - handle-keyboard-layout-changed! fired for each switch"

r=$(seval '(set-keyboard-layout! 7)')
sleep 0.5
[ "$(active)" = '"German (Bone)"' ] || fail "out-of-range index changed the group"
echo "ok - out-of-range index is ignored"

nested_stop

# Invalid requested names must not be reused by the fallback. In particular,
# an empty fallback variant normally inherits XKB_DEFAULT_VARIANT again.
for bad in layout variant rules; do
    export NESTED_XKB_LAYOUT=us NESTED_XKB_VARIANT='' NESTED_XKB_RULES=''
    export NESTED_XKB_MODEL=pc105 NESTED_XKB_OPTIONS=caps:swapescape
    case "$bad" in
        layout) export NESTED_XKB_LAYOUT=minde-invalid-layout ;;
        variant) export NESTED_XKB_VARIANT=minde-invalid-variant ;;
        rules)
            export NESTED_XKB_RULES=minde-invalid-rules
            export NESTED_XKB_MODEL=minde-invalid-model
            ;;
    esac
    nested_start "$OUT/bad-$bad" "${MINDE_KEYBOARD_LAYOUT_E2E_DISPLAY:-:99}" ||
        fail "invalid $bad prevented startup" "$NESTED_LOG"
    [ "$(active)" = '"English (US)"' ] || fail "fallback is not US: $(active)"
    [ "$(seval '(length (keyboard-layouts))')" = 1 ] || fail "fallback has extra groups"
    nested_log_has 'falling back to the US keymap' || fail "fallback diagnostic missing"
    for name in RULES MODEL LAYOUT VARIANT OPTIONS; do
        nested_log_has "XKB_DEFAULT_$name=" || fail "diagnostic omits XKB_DEFAULT_$name"
    done
    nested_log_has "minde-invalid-$bad" || fail "diagnostic omits invalid $bad value"
    if nested_log_has "panicked at"; then fail "invalid $bad caused a panic"; fi

    # Drive actual host key events through the recovered compositor keymap.
    # Caps_Lock must retain its default meaning despite caps:swapescape above.
    seval '(begin (define %fallback-keys (quote ()))
                  (define (wm-handle-key mods sym name text)
                    (set! %fallback-keys (cons name %fallback-keys)) #t) #t)' >/dev/null
    window=$(timeout 5 xdotool search --sync --name Smithay | head -n 1)
    timeout 5 xdotool windowfocus "$window"
    timeout 5 xdotool key y Caps_Lock Caps_Lock
    attempt=0
    until [ "$(seval '%fallback-keys')" = '("Caps_Lock" "Caps_Lock" "y")' ]; do
        attempt=$((attempt + 1))
        [ "$attempt" -le 30 ] || fail "fallback keys/options are incorrect: $(seval '%fallback-keys')"
        sleep 0.1
    done
    echo "ok - invalid $bad logs requested values and recovers with working US input"
    nested_stop
done

# Corrupt installed keymap data can defeat both attempts. Fail before starting
# a desktop, with a useful error rather than an unwrap panic. The invalid
# evdev rules take precedence over any host/user include directories.
bad_root="$OUT/bad-data"
mkdir -p "$bad_root/xkb/rules" "$bad_root/runtime"
chmod 700 "$bad_root/runtime"
printf '%s\n' 'not a valid rules file' >"$bad_root/xkb/rules/evdev"
status=0
timeout 10 env XDG_CONFIG_HOME="$bad_root" XDG_RUNTIME_DIR="$bad_root/runtime" \
    XDG_STATE_HOME="$bad_root/state" \
    XKB_CONFIG_ROOT="$bad_root/xkb" XKB_DEFAULT_RULES=evdev \
    XKB_DEFAULT_LAYOUT=us XKB_DEFAULT_VARIANT='' XKB_DEFAULT_MODEL=pc105 \
    XKB_DEFAULT_OPTIONS='' "$NESTED_BINARY" --winit >"$bad_root/compositor.log" 2>&1 || status=$?
[ "$status" -eq 1 ] || fail "unavailable fallback exited with $status" "$bad_root/compositor.log"
NESTED_LOG="$bad_root/compositor.log"
nested_log_has 'could not initialize the fallback US keymap' || fail "fallback failure diagnostic missing"
if nested_log_has 'panicked at'; then fail "fallback failure panicked"; fi
echo "ok - unavailable fallback produces a clean startup error"
echo "keyboard layout e2e: all checks passed"
