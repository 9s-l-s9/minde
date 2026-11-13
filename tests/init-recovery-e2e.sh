#!/bin/sh
# SPDX-License-Identifier: GPL-3.0-or-later
# Exercise real startup failures, clean fallback, and policy-independent exit.
set -eu
cd "$(dirname "$0")/.."
. tests/lib/nested-compositor.sh
export MINDE_TEST_PROTOCOLS=1
OUT=${MINDE_INIT_RECOVERY_E2E_OUT:-/tmp/minde-init-recovery}
nested_install_cleanup nested_stop
mkdir -p "$OUT/fixtures"
printf '(\n' > "$OUT/fixtures/syntax.scm"
printf ';; failing source location\n\n(minde-init-unbound-variable)\n' > "$OUT/fixtures/unbound.scm"
printf '(use-modules (minde nonexistent-init-module))\n' > "$OUT/fixtures/module.scm"
cat > "$OUT/fixtures/partial.scm" <<'SCM'
(load-from-path "init")
(define minde-dirty-init #t)
(add-event-hook! 'focus-window (lambda _ (error "leftover user hook")))
(setenv "MINDE_DIRTY_INIT" "yes")
(setenv "MINDE_CONFIG" "/nonexistent-dirty-config.scm")
(chdir "/")
(wm-quit)
(error "partial-init-failure")
SCM

fail() { echo "error: $*; inspect $OUT" >&2; exit 1; }
seval() { timeout 10 scripts/mindectl eval "$1"; }

for case in syntax unbound module partial; do
    export MINDE_NESTED_INIT="$OUT/fixtures/$case.scm"
    export NESTED_XKB_OPTIONS=''
    [ "$case" != partial ] || export NESTED_XKB_OPTIONS=terminate:ctrl_alt_bksp
    nested_start "$OUT/$case" "${MINDE_INIT_RECOVERY_E2E_DISPLAY:-:94}" ||
        fail "$case did not recover"
    # A partial user init may have logged 'scheme layer loaded' before failing.
    # Wait for the *recovered* backend to start, not that earlier load marker.
    attempt=0
    until nested_log_has 'recovered policy using bundled init'; do
        attempt=$((attempt + 1))
        [ "$attempt" -le 200 ] || fail "$case recovery timed out"
        kill -0 "$NESTED_WM_PID" 2>/dev/null || fail "$case process exited"
        sleep 0.1
    done
    [ "$(grep -c 'selected backend' "$NESTED_LOG")" = 2 ] || fail 'expected exactly one restart'
    nested_log_has "$case.scm" || fail 'missing source path'
    case "$case" in
        syntax)
            nested_log_has 'read-error' || fail 'missing syntax diagnostic'
            nested_log_has 'syntax.scm:2:' || fail 'missing read-error line'
            ;;
        unbound)
            nested_log_has 'unbound-variable' || fail 'missing exception key'
            nested_log_has 'unbound.scm:3:' || fail 'missing interpreted source line'
            ;;
        module) nested_log_has 'nonexistent-init-module' || fail 'missing module diagnostic' ;;
        partial) nested_log_has 'partial-init-failure' || fail 'missing exception arguments' ;;
    esac
    [ "$(seval '(and (not (defined? (quote minde-dirty-init)))
                     (not (getenv "MINDE_DIRTY_INIT"))
                     (not (getenv "_MINDE_INIT_RECOVERY")))')" = '#t' ] ||
        fail 'partial policy or recovery environment survived'
    [ "$(seval '(null? (event-hook-procedures (quote focus-window)))')" = '#t' ] ||
        fail 'partial module state survived'
    seval '(begin (define recovery-key-fired #f)
                  (bind-key! (quote ()) "F5" (lambda () (set! recovery-key-fired #t))))' >/dev/null
    window=$(timeout 5 xdotool search --sync --name Smithay | head -n 1)
    xdotool windowfocus "$window"
    xdotool key F5
    attempt=0
    until [ "$(seval 'recovery-key-fired')" = '#t' ]; do
        attempt=$((attempt + 1))
        [ "$attempt" -le 30 ] || fail 'recovered keybindings do not work'
        sleep 0.1
    done
    nested_capture "$OUT/$case/recovery.png"
    # Break the policy callback after startup, then exit through actual host
    # keyboard events. A related chord must remain an ordinary client key.
    seval '(define (wm-handle-key . args) (error "broken-key-policy"))' >/dev/null
    xdotool key ctrl+BackSpace
    seval '#t' >/dev/null
    if [ "$case" = partial ]; then
        # Also cover a focused lock surface and XKB's Terminate_Server symbol.
        timeout 15 "$NESTED_PROTOCOL_CLIENT" emergency-exit
    else
        xdotool key ctrl+alt+BackSpace
    fi
    attempt=0
    while kill -0 "$NESTED_WM_PID" 2>/dev/null; do
        attempt=$((attempt + 1))
        [ "$attempt" -le 50 ] || fail 'emergency exit did not stop the compositor'
        sleep 0.1
    done
    wait "$NESTED_WM_PID" || fail 'emergency exit was not clean'
    nested_log_has 'emergency exit' || fail 'missing exit diagnostic'
    echo "ok - $case recovers clean policy, working keys and emergency exit"
    nested_stop
done

# Declarative emergency settings also apply before the backend exists. Keep
# XKB's terminate alias enabled while testing disable/rebind and failed reloads.
unset MINDE_NESTED_INIT
export NESTED_XKB_OPTIONS=terminate:ctrl_alt_bksp
export MINDE_EMERGENCY_CONFIG="$OUT/fixtures/emergency.scm"
export MINDE_NESTED_CONFIG="$MINDE_EMERGENCY_CONFIG"
for case in inhibited locked; do
    cat > "$MINDE_EMERGENCY_CONFIG" <<'SCM'
(minde-config (version 1) (prefix (ctrl) "t") (emergency-exit #f) (bindings))
SCM
    nested_start "$OUT/$case" "${MINDE_INIT_RECOVERY_E2E_DISPLAY:-:94}" ||
        fail "$case configuration did not start"
    window=$(timeout 5 xdotool search --sync --name Smithay | head -n 1)
    xdotool windowfocus "$window"
    timeout 30 "$NESTED_PROTOCOL_CLIENT" emergency-config "$case"
    attempt=0
    while kill -0 "$NESTED_WM_PID" 2>/dev/null; do
        attempt=$((attempt + 1))
        [ "$attempt" -le 50 ] || fail 'configured emergency exit did not stop the compositor'
        sleep 0.1
    done
    wait "$NESTED_WM_PID" || fail 'configured emergency exit was not clean'
    nested_log_has 'emergency exit' || fail 'missing configured exit diagnostic'
    nested_stop
done
unset MINDE_NESTED_CONFIG MINDE_EMERGENCY_CONFIG NESTED_XKB_OPTIONS

# A broken bundle is fatal before backend setup. Aliases of the same file must
# not trigger even one retry; distinct user+bundle failures stop after one.
mkdir -p "$OUT/broken-bundle" "$OUT/fatal-runtime"
chmod 700 "$OUT/fatal-runtime"
printf '(error "broken-bundled-init")\n' > "$OUT/broken-bundle/init.scm"
ln -sf "$OUT/broken-bundle/init.scm" "$OUT/fixtures/bundle-alias.scm"
for case in bundle-alias unbound; do
    code=0
    timeout 15 env XDG_RUNTIME_DIR="$OUT/fatal-runtime" \
        MINDE_SCHEME_DIR="$OUT/broken-bundle" MINDE_INIT="$OUT/fixtures/$case.scm" \
        "$NESTED_BINARY" --winit > "$OUT/fatal-$case.log" 2>&1 || code=$?
    [ "$code" = 1 ] || fail "$case should fail promptly, got exit $code"
    grep -q 'bundled init failed; aborting startup' "$OUT/fatal-$case.log" || fail 'missing fatal diagnostic'
    expected=1
    [ "$case" != unbound ] || expected=2
    [ "$(grep -c 'selected backend' "$OUT/fatal-$case.log")" = "$expected" ] || fail 'unexpected retry count'
    echo "ok - $case stops when bundled init fails"
done
