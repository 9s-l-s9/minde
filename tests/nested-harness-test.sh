#!/bin/sh
# SPDX-License-Identifier: GPL-3.0-or-later
# Headless lifecycle checks; "display" and "scenario" exercise private servers.
set -eu
cd "$(dirname "$0")/.."
if [ "${1:-all}" = all ]; then
    failed=0
    for case_name in log stop mktemp mkdir chmod signals client locker; do
        sh tests/nested-harness-test.sh "$case_name" || failed=1
    done
    exit "$failed"
fi
. tests/lib/nested-compositor.sh
scratch=$(mktemp -d "${TMPDIR:-/tmp}/minde-harness-test.XXXXXX")
holder_pid=
scenario_driver=
cleanup() {
    if [ -n "$scenario_driver" ]; then
        kill "$scenario_driver" 2>/dev/null || true
        wait "$scenario_driver" 2>/dev/null || true
    fi
    nested_stop
    if [ -n "$holder_pid" ]; then
        kill "$holder_pid" 2>/dev/null || true
        wait "$holder_pid" 2>/dev/null || true
    fi
    rm -rf "$scratch"
}
trap cleanup EXIT
trap 'exit 1' HUP INT TERM
fail() { echo "FAIL - $1" >&2; exit 1; }

case "$1" in
log)
    NESTED_LOG="$scratch/log"
    printf 'old event\nmanaged toplevel mapped \033[3mprotocol\033[0m\033[2m=\033[0m"wayland"\n' >"$NESTED_LOG"
    NESTED_WM_PID=
    nested_log_has 'protocol="wayland"' || fail 'whole log misses a colored field'
    nested_wait_for_log_after 2 'protocol="wayland"' 1 || fail 'new log lines miss a colored field'
    if nested_wait_for_log_after 3 'protocol="wayland"' 1; then
        fail 'log wait matched an older event'
    fi
    ;;
stop)
    # Fake IDs make duplicate signalling observable without risking PID reuse.
    kill() { printf '%s\n' "$*" >>"$scratch/signals"; }
    wait() { printf '%s\n' "$*" >>"$scratch/waits"; }
    NESTED_WM_PID=111
    NESTED_XVFB_PID=222
    NESTED_RT=$(mktemp -d "$scratch/runtime.XXXXXX")
    old_runtime=$NESTED_RT
    nested_stop
    nested_stop
    [ "$(wc -l <"$scratch/signals")" -eq 2 ] || fail 'teardown signals retired PIDs again'
    [ "$(wc -l <"$scratch/waits")" -eq 2 ] || fail 'teardown waits on retired PIDs again'
    [ -z "$NESTED_WM_PID$NESTED_XVFB_PID$NESTED_RT" ] || fail 'teardown retains owned resources'
    [ ! -d "$old_runtime" ] || fail 'runtime directory survived teardown'
    ;;
mktemp|mkdir|chmod)
    mkdir -p "$scratch/out"
    cargo() { return 0; }
    jq() { return 0; }
    xdpyinfo() { return 1; }
    Xvfb() { touch "$scratch/started"; }
    case "$1" in
        mktemp) mktemp() { return 1; }; mkdir() { return 1; }; chmod() { return 1; } ;;
        mkdir) mkdir() { return 1; } ;;
        chmod) chmod() { return 1; } ;;
    esac
    if nested_start "$scratch/out" :invalid 2>"$scratch/error"; then
        fail 'startup ignored a filesystem failure'
    fi
    [ ! -e "$scratch/started" ] || fail 'startup launched a display after a filesystem failure'
    ;;
client)
    NESTED_RT=$(mktemp -d "$scratch/runtime.XXXXXX")
    NESTED_WAYLAND_DISPLAY=unused
    # shellcheck disable=SC2016
    nested_wayland_exec sh -c 'printf "%s\n" "$$" >"$1"; exec sleep 30' sh "$scratch/client" &
    NESTED_WM_PID=$!
    attempt=0
    while [ ! -s "$scratch/client" ]; do
        attempt=$((attempt + 1))
        [ "$attempt" -le 20 ] || fail 'background client did not start'
        sleep 0.05
    done
    [ "$(cat "$scratch/client")" = "$NESTED_WM_PID" ] || fail 'background PID names a wrapper shell'
    ;;
locker)
    printf 'pam_start failed\n' >"$scratch/locker"
    nested_locker_unavailable "$scratch/locker" || fail 'PAM startup failure was not recognized'
    for reason in 'Failed to lock session' 'Missing ext-session-lock-v1' ''; do
        printf '%s\n' "$reason" >"$scratch/locker"
        if nested_locker_unavailable "$scratch/locker"; then fail 'protocol failure became an environment skip'; fi
    done
    ;;
signals)
    cat >"$scratch/child.sh" <<'SH'
set -eu
. "$1/tests/lib/nested-compositor.sh"
nested_install_cleanup
NESTED_RT=$(mktemp -d "$2/runtime.XXXXXX")
sleep 30 & NESTED_WM_PID=$!
sleep 30 & NESTED_XVFB_PID=$!
printf '%s %s %s\n' "$NESTED_WM_PID" "$NESTED_XVFB_PID" "$NESTED_RT" >"$2/owned"
kill -s "$3" "$$"
touch "$2/resumed"
SH
    for signal_status in HUP:129 INT:130 TERM:143; do
        status=0
        sh "$scratch/child.sh" "$PWD" "$scratch" "${signal_status%:*}" || status=$?
        [ "$status" -eq "${signal_status#*:}" ] || fail "wrong exit status after $signal_status: $status"
        read -r NESTED_WM_PID NESTED_XVFB_PID NESTED_RT <"$scratch/owned"
        [ ! -e "$scratch/resumed" ] || fail 'scenario resumed after a signal'
        for owned in "$NESTED_WM_PID" "$NESTED_XVFB_PID"; do
            if kill -0 "$owned" 2>/dev/null; then fail 'signal cleanup left a child alive'; fi
        done
        [ ! -d "$NESTED_RT" ] || fail 'signal cleanup retained the runtime directory'
        NESTED_WM_PID='' NESTED_XVFB_PID='' NESTED_RT=''
    done
    ;;
display)
    NESTED_OUT="$scratch/display"
    mkdir -p "$NESTED_OUT"
    NESTED_DISPLAY=${MINDE_HARNESS_DISPLAY:-:639}
    nested_start_xvfb || fail 'private Xvfb did not start'
    holder_pid=$NESTED_XVFB_PID
    NESTED_XVFB_PID=
    if nested_start_xvfb; then fail 'startup accepted an occupied display'; fi
    nested_stop
    kill -0 "$holder_pid" || fail 'collision cleanup killed the original server'
    ;;
scenario)
    # Build before the bounded startup wait, including on a cold checkout.
    if [ -z "${MINDE_NESTED_PACKAGE:-}" ]; then
        # Use the driver's environment, independent of packaged child overrides.
        # shellcheck disable=SC2031
        LD_LIBRARY_PATH="${GUIX_ENVIRONMENT:-}/lib${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}" \
            cargo build >"$scratch/build.log" 2>&1 || fail 'compositor build failed'
    fi
    mkdir -p "$scratch/bin"
    cat >"$scratch/bin/import" <<'SH'
#!/bin/sh
printf '%s\n' "$@" >"$MINDE_HARNESS_RECORD.import"
exit 17
SH
    chmod +x "$scratch/bin/import"
    cat >"$scratch/init.scm" <<'SCM'
(load-from-path "init")
(call-with-output-file (string-append (getenv "MINDE_HARNESS_RECORD") ".runtime")
  (lambda (port) (display (getenv "XDG_RUNTIME_DIR") port) (newline port)))
(call-with-output-file (string-append (getenv "MINDE_HARNESS_RECORD") ".pid")
  (lambda (port) (display (getpid) port) (newline port)))
SCM
    for stop_mode in failure TERM; do
        record="$scratch/$stop_mode"
        display=${MINDE_HARNESS_DISPLAY:-:643}
        env PATH="$scratch/bin:$PATH" MINDE_NESTED_INIT="$scratch/init.scm" \
            MINDE_HARNESS_RECORD="$record" E2E_OUT="$scratch/scenario" E2E_DISPLAY="$display" \
            sh tests/e2e.sh >"$scratch/driver.log" 2>&1 &
        scenario_driver=$!
        attempt=0
        while [ ! -s "$record.pid" ]; do
            kill -0 "$scenario_driver" 2>/dev/null || { cat "$scratch/driver.log"; fail 'scenario died before startup'; }
            attempt=$((attempt + 1))
            [ "$attempt" -le 600 ] || fail 'scenario did not start within one minute'
            sleep 0.1
        done
        read -r NESTED_WM_PID <"$record.pid"
        IFS= read -r NESTED_RT <"$record.runtime"
        # With -displayfd there need not be an X lock file. Inspect only the
        # driver's own direct children, never a global process-name search.
        NESTED_XVFB_PID=
        children=$(cat "/proc/$scenario_driver/task/$scenario_driver/children")
        for candidate in $children; do
            if [ "$(cat "/proc/$candidate/comm" 2>/dev/null)" = Xvfb ]; then
                NESTED_XVFB_PID=$candidate
            fi
        done
        [ -n "$NESTED_XVFB_PID" ] || fail 'scenario has no owned Xvfb child'
        if [ "$stop_mode" = TERM ]; then kill -TERM "$scenario_driver"; fi
        attempt=0
        while kill -0 "$scenario_driver" 2>/dev/null; do
            attempt=$((attempt + 1))
            [ "$attempt" -le 200 ] || fail 'scenario did not stop within twenty seconds'
            sleep 0.1
        done
        status=0
        wait "$scenario_driver" || status=$?
        scenario_driver=
        for owned in "$NESTED_WM_PID" "$NESTED_XVFB_PID"; do
            if kill -0 "$owned" 2>/dev/null; then fail "$stop_mode left a compositor or display alive"; fi
        done
        [ ! -d "$NESTED_RT" ] || fail "$stop_mode retained the runtime directory"
        NESTED_WM_PID='' NESTED_XVFB_PID='' NESTED_RT=''
        expected=143
        if [ "$stop_mode" = failure ]; then
            # The Scheme driver reports helper failures through its common
            # assertion exit. Prove this was our injected failure, rather
            # than accepting an unrelated earlier assertion with status 1.
            [ -s "$record.import" ] || fail 'scenario never reached the injected screenshot failure'
            grep -Fq 'command failed ("import" ' "$scratch/driver.log" ||
                fail 'scenario failed for a reason other than the screenshot helper'
            expected=1
        fi
        [ "$status" -eq "$expected" ] || { cat "$scratch/driver.log"; fail "$stop_mode returned $status, expected $expected"; }
        echo "ok - main e2e cleans up after $stop_mode"
    done
    ;;
*) fail 'unknown harness test case' ;;
esac
echo "ok - nested harness $1"
