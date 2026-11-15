#!/bin/sh
# SPDX-License-Identifier: GPL-3.0-or-later
# Historical keymap integration under a private Xvfb/winit session. Scheme
# asserts observable effects; shell owns processes and external diagnostics.
set -eu
cd "$(dirname "$0")/.."
. tests/lib/nested-compositor.sh

nested_output_directory "${E2E_OUT:-}" e2e
LOG=$OUT/compositor.log
SWAYLOCK_PID=
SUBSCRIBE_PID=
STRESS_PIDS=
cleanup() {
    for pid in "$SWAYLOCK_PID" "$SUBSCRIBE_PID" $STRESS_PIDS; do
        [ -n "$pid" ] || continue
        kill "$pid" 2>/dev/null || true
        wait "$pid" 2>/dev/null || true
    done
    nested_stop
}
nested_install_cleanup cleanup
fail() {
    echo "FAIL - $*; artifacts: $OUT" >&2
    [ ! -f "$LOG" ] || tail -n 8 "$LOG" >&2
    exit 1
}
ok() { echo "ok - $*"; }
loggrep() { nested_log_has "$1"; }
# Preserve errors as well as values, including commands in the lock block.
eval_checked() {
    printf '\nrequest: %s\n' "$1" >>"$OUT/cli-ipc.log"
    if scripts/mindectl eval "$1" >"$OUT/cli-reply" 2>>"$OUT/cli-ipc.log"; then
        cat "$OUT/cli-reply" >>"$OUT/cli-ipc.log"
        cat "$OUT/cli-reply"
    else
        cat "$OUT/cli-reply" >>"$OUT/cli-ipc.log"
        fail "IPC evaluation failed; see cli-ipc.log"
    fi
}
for tool in foot xdotool import guile; do
    command -v "$tool" >/dev/null 2>&1 || fail "$tool is required"
done
export MINDE_NESTED_LEGACY_KEYMAP=1
nested_start "$OUT" "${E2E_DISPLAY:-:97}" || fail 'nested compositor startup'
: >"$OUT/cli-ipc.log"
timeout 5 xdotool search --sync --name Smithay windowfocus || fail 'focus private compositor'
timeout 180 guile --no-auto-compile -L scheme tests/e2e.scm "$OUT" || fail 'behavioral assertions (see ipc.scm)'

scripts/mindectl query state --json >"$OUT/state.json" || fail 'structured state query'
jq -e '.schema_version == 1 and .runtime.backend == "winit"' "$OUT/state.json" >/dev/null || fail 'state schema/backend'
[ -s "$NESTED_RT/minde/status.json" ] || fail 'atomic status file missing'
timeout 5 scripts/mindectl subscribe --json >"$OUT/subscription.json" 2>"$OUT/subscription.err" &
SUBSCRIBE_PID=$!
attempt=0
until [ -s "$OUT/subscription.json" ] && jq -e '.schema_version == 1' "$OUT/subscription.json" >/dev/null 2>&1; do
    kill -0 "$SUBSCRIBE_PID" 2>/dev/null || fail 'status subscription exited without data'
    attempt=$((attempt + 1)); [ "$attempt" -lt 50 ] || fail 'status subscription timeout'
    sleep 0.05
done
kill "$SUBSCRIBE_PID" 2>/dev/null || true
wait "$SUBSCRIBE_PID" 2>/dev/null || true
SUBSCRIBE_PID=
jq -e '.schema_version == 1' "$OUT/subscription.json" >/dev/null || fail 'subscription schema'
REPORT=$OUT/report-$NESTED_WM_PID
scripts/mindectl report --output "$REPORT" >"$OUT/report.log" 2>&1 || fail 'diagnostic report'
grep -q '^report_schema_version=1$' "$REPORT/report.txt" || fail 'report schema'
jq -e '.schema_version == 1' "$REPORT/state.json" >/dev/null || fail 'report state'
grep -q '"title"\|"app_id"' "$REPORT/state.json" && fail 'report leaked window metadata'
grep -q '^(' "$REPORT/timing.txt" || fail 'report timing'
[ "$(cat "$REPORT/drift.txt")" = '()' ] || fail 'policy mirrors drifted from compositor'
# The driver's HOME deliberately differs from the private compositor's HOME.
# shellcheck disable=SC2031
grep -Fq "$HOME" "$REPORT"/* && fail 'report leaked the driver home path'
ok 'versioned status, subscription and redacted diagnostic report'

if [ -n "${MINDE_E2E_STRESS:-}" ]; then
    worker() {
        n=0
        while [ "$n" -lt 15 ]; do
            scripts/mindectl eval '(begin (unless (reload-configuration!) (error "reload failed")) (current-group-name))' || return 1
            n=$((n + 1))
        done
    }
    for index in 1 2 3; do
        worker >"$OUT/stress-$index.log" 2>&1 &
        STRESS_PIDS="$STRESS_PIDS $!"
    done
    for pid in $STRESS_PIDS; do wait "$pid" || fail 'concurrent IPC/reload stress'; done
    STRESS_PIDS=
    ok 'concurrent IPC/reload stress'
fi

# Real PAM authentication is not automated. Native lock/unlock/recovery is
# covered by session-lock-e2e.sh; this last block checks the external locker.
if command -v wayland-info >/dev/null 2>&1; then
    nested_wayland timeout 5 wayland-info >"$OUT/wayland-info" 2>"$OUT/wayland-info.err" || fail 'Wayland global query'
    grep -q ext_session_lock_manager_v1 "$OUT/wayland-info" || fail 'lock global missing'
fi
if command -v swaylock >/dev/null 2>&1; then
    # Keep recovery inside the disposable session deterministic: the separate
    # native lock suite verifies automatic replacement; here inspect owner loss.
    eval_checked '(begin (set! %lock-restart? #f) #t)' >/dev/null
    line=$(($(wc -l <"$LOG") + 1))
    nested_wayland_exec swaylock >"$OUT/swaylock.log" 2>&1 &
    SWAYLOCK_PID=$!
    if nested_wait_for_log_after "$line" 'session locked (ext-session-lock)' 8; then
        [ "$(eval_checked '(wm-session-locked?)')" = '#t' ] || fail 'locker did not lock'
        reloads_before=$(nested_plain_log <"$LOG" | grep -ac 'reloaded ' || true)
        timeout 5 xdotool key --clearmodifiers Print shift+r
        # Deliberate observation interval for a forbidden event, not command synchronisation.
        sleep 1
        reloads_after=$(nested_plain_log <"$LOG" | grep -ac 'reloaded ' || true)
        [ "$reloads_before" = "$reloads_after" ] || fail 'reload keys reached policy while locked'
        import -window root "$OUT/locked.png"
        line=$(($(wc -l <"$LOG") + 1))
        kill "$SWAYLOCK_PID" || fail 'locker exited before owner-loss check'
        wait "$SWAYLOCK_PID" 2>/dev/null || true
        SWAYLOCK_PID=
        nested_wait_for_log_after "$line" 'session lock owner disappeared' 5 || fail 'locker loss not observed'
        [ "$(eval_checked '(wm-session-locked?)')" = '#t' ] || fail 'locker death unlocked session'
        ok 'external locker locks, filters shortcuts and keeps session locked after owner loss'
    elif ! kill -0 "$SWAYLOCK_PID" 2>/dev/null && nested_locker_unavailable "$OUT/swaylock.log"; then
        echo 'skip - swaylock PAM/account unavailable; see swaylock.log'
    else
        fail 'external locker did not confirm lock'
    fi
else
    echo 'skip - swaylock not installed; native lock suite supplies protocol coverage'
fi
kill -0 "$NESTED_WM_PID" || fail 'compositor exited'
loggrep 'panicked at\|error in keybinding\|error in timer' && fail 'runtime error'
echo 'e2e: all behavioral assertions passed'
