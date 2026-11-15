#!/bin/sh
# SPDX-License-Identifier: GPL-3.0-or-later
# Shared nested-compositor lifecycle used by application and soak scenarios.

# Ownership belongs to this driver, never to an inherited shell environment.
NESTED_WM_PID='' NESTED_XVFB_PID='' NESTED_RT='' NESTED_WAYLAND_DISPLAY=''
NESTED_OUT_AUTOMATIC='' NESTED_OUT_KEEP='' NESTED_CLEANUP=nested_stop

nested_install_cleanup() {
    # Capture the caller's cleanup function now, before positional args change.
    NESTED_CLEANUP=${1:-nested_stop}
    trap 'nested_exit "$?"' EXIT
    trap 'exit 129' HUP
    trap 'exit 130' INT
    trap 'exit 143' TERM
}

nested_exit() {
    "$NESTED_CLEANUP"
    nested_finish_output "$1"
}

# Set OUT to the caller's override ($1) or to a private directory for this
# run named after $2. A shared fixed path would let concurrent runs and other
# users replace logs and compiled helpers. With "keep" as $3 the directory
# holds results and survives a successful run.
nested_output_directory() {
    NESTED_OUT_AUTOMATIC='' NESTED_OUT_KEEP=${3:-}
    if [ -n "$1" ]; then
        OUT=$1
    else
        OUT=$(mktemp -d "${TMPDIR:-/tmp}/minde-test-$2.XXXXXX") || return 1
        NESTED_OUT_AUTOMATIC=$OUT
    fi
}

# Remove an automatic directory after success; report where the logs of a
# failed run, or the retained results, can be found.
nested_finish_output() {
    [ -n "${NESTED_OUT_AUTOMATIC:-}" ] || return 0
    if [ "$1" -eq 0 ] && [ -z "${NESTED_OUT_KEEP:-}" ]; then
        rm -rf "$NESTED_OUT_AUTOMATIC"
    else
        echo "test output: $NESTED_OUT_AUTOMATIC" >&2
    fi
    NESTED_OUT_AUTOMATIC=''
}

nested_plain_log() {
    sed "s/$(printf '\033')\[[0-9;]*m//g"
}

nested_log_has() {
    nested_plain_log <"$NESTED_LOG" | grep -aq "$1"
}

nested_locker_unavailable() {
    # A protocol failure or an unexplained exit is a test failure, not a skip.
    grep -Eq 'pam_start failed|getpwuid failed' "$1"
}

nested_stop() {
    set -- "${NESTED_WM_PID:-}" "${NESTED_XVFB_PID:-}"
    NESTED_WM_PID='' NESTED_XVFB_PID=''
    for nested_pid do
        if [ -n "$nested_pid" ]; then
            kill "$nested_pid" 2>/dev/null || true
            wait "$nested_pid" 2>/dev/null || true
        fi
    done
    if [ -n "${NESTED_RT:-}" ] && [ -d "$NESTED_RT" ]; then
        rm -rf "$NESTED_RT"
    fi
    NESTED_RT='' NESTED_WAYLAND_DISPLAY=''
}

nested_start_xvfb() {
    # Only our server can acknowledge readiness through this descriptor.
    # A socket probe could instead reach an existing server on a busy display.
    : >"$NESTED_OUT/xvfb-ready" || return 1
    Xvfb "$NESTED_DISPLAY" -displayfd 3 -screen 0 1280x800x24 \
        3>"$NESTED_OUT/xvfb-ready" >"$NESTED_OUT/xvfb.log" 2>&1 &
    NESTED_XVFB_PID=$!
    nested_attempt=0
    while :; do
        kill -0 "$NESTED_XVFB_PID" 2>/dev/null || return 1
        if [ -s "$NESTED_OUT/xvfb-ready" ]; then
            read -r nested_actual_display <"$NESTED_OUT/xvfb-ready" || return 1
            # Check the server's own display number before exporting DISPLAY.
            [ ":$nested_actual_display" = "$NESTED_DISPLAY" ] || return 1
            return 0
        fi
        nested_attempt=$((nested_attempt + 1))
        [ "$nested_attempt" -le 20 ] || return 1
        sleep 0.1
    done
}

nested_start() {
    NESTED_OUT=$1
    NESTED_DISPLAY=$2
    NESTED_RT=$(mktemp -d "${TMPDIR:-/tmp}/minde-nested-runtime.XXXXXX") || return 1
    NESTED_LOG="$NESTED_OUT/compositor.log"
    mkdir -p "$NESTED_OUT" || return 1
    mkdir -p "$NESTED_RT/home" "$NESTED_RT/cache" "$NESTED_RT/config" \
        "$NESTED_RT/state" || return 1
    chmod 700 "$NESTED_RT" || return 1
    : >"$NESTED_LOG" || return 1

    if [ -z "${MINDE_NESTED_PACKAGE:-}" ] || [ "${MINDE_TEST_PROTOCOLS:-0}" = 1 ]; then
        command -v cargo >/dev/null 2>&1 || {
            echo "error: cargo is required; enter the project Guix shell" >&2
            return 127
        }
    fi
    command -v Xvfb >/dev/null 2>&1 || {
        echo "error: Xvfb is required; add xorg-server" >&2
        return 127
    }
    command -v jq >/dev/null 2>&1 || {
        echo "error: jq is required for structured scenario assertions" >&2
        return 127
    }

    nested_start_xvfb || return 1

    export DISPLAY="$NESTED_DISPLAY"
    export XDG_RUNTIME_DIR="$NESTED_RT"
    # A deterministic keymap regardless of the host session; a gate that
    # needs custom names sets NESTED_XKB_* (tests/keyboard-layout-e2e.sh).
    export XKB_DEFAULT_LAYOUT="${NESTED_XKB_LAYOUT:-us}"
    unset XKB_DEFAULT_VARIANT XKB_DEFAULT_OPTIONS XKB_DEFAULT_MODEL XKB_DEFAULT_RULES
    [ -z "${NESTED_XKB_VARIANT:-}" ] || export XKB_DEFAULT_VARIANT="$NESTED_XKB_VARIANT"
    [ -z "${NESTED_XKB_RULES:-}" ] || export XKB_DEFAULT_RULES="$NESTED_XKB_RULES"
    [ -z "${NESTED_XKB_MODEL:-}" ] || export XKB_DEFAULT_MODEL="$NESTED_XKB_MODEL"
    [ -z "${NESTED_XKB_OPTIONS:-}" ] || export XKB_DEFAULT_OPTIONS="$NESTED_XKB_OPTIONS"
    unset MINDE_REPL_STARTED WAYLAND_DISPLAY MINDE_FULL_KEYMAP \
        MINDE_E2E_LEGACY_KEYMAP
    export LD_LIBRARY_PATH="${GUIX_ENVIRONMENT:-/nonexistent}/lib${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
    export RUST_LOG=minde=debug,scheme=info

    if [ -n "${MINDE_NESTED_PACKAGE:-}" ]; then
        NESTED_BINARY="$MINDE_NESTED_PACKAGE/bin/minde"
        NESTED_SCHEME="$MINDE_NESTED_PACKAGE/share/minde/scheme"
        [ -x "$NESTED_BINARY" ] || return 1
    else
        cargo build >"$NESTED_OUT/build.log" 2>&1 || return 1
        NESTED_BINARY="$PWD/target/debug/minde"
        NESTED_SCHEME="$PWD/scheme"
    fi
    if [ "${MINDE_TEST_PROTOCOLS:-0}" = 1 ]; then
        cargo build --example protocol-client >"$NESTED_OUT/protocol-build.log" 2>&1 || return 1
        NESTED_PROTOCOL_CLIENT="$PWD/target/debug/examples/protocol-client"
        export NESTED_PROTOCOL_CLIENT
    fi
    (
        if [ -n "${MINDE_NESTED_PACKAGE:-}" ]; then
            # Test the packaged runtime and its own library search paths.
            unset LD_LIBRARY_PATH
            export GUILE_LOAD_PATH="$MINDE_NESTED_PACKAGE/share/guile/site/3.0"
            export GUILE_LOAD_COMPILED_PATH="$MINDE_NESTED_PACKAGE/lib/guile/3.0/site-ccache"
            export GUILE_AUTO_COMPILE=0
        fi
        # A host autocompile cache can hide a missing packaged init.go.
        export HOME="$NESTED_RT/home"
        export XDG_CACHE_HOME="$NESTED_RT/cache"
        export XDG_CONFIG_HOME="$NESTED_RT/config"
        export XDG_STATE_HOME="$NESTED_RT/state"
        if [ "${MINDE_NESTED_LEGACY_KEYMAP:-0}" = 1 ]; then
            export MINDE_E2E_LEGACY_KEYMAP=1
        fi
        export MINDE_INIT="${MINDE_NESTED_INIT:-$NESTED_SCHEME/init.scm}"
        export MINDE_SCHEME_DIR="$NESTED_SCHEME"
        export MINDE_CONFIG="${MINDE_NESTED_CONFIG:-$PWD/tests/e2e-config.scm}"
        export MINDE_RULES_FILE="$NESTED_OUT/rules.scm"
        export MINDE_LAYOUTS_FILE="$NESTED_OUT/layouts.scm"
        # Fault-injection libraries belong only to the compositor process,
        # never Cargo, Xvfb, clients or tools run by the test driver.
        if [ -n "${MINDE_NESTED_LIBRARY_PATH:-}" ]; then
            export LD_LIBRARY_PATH="$MINDE_NESTED_LIBRARY_PATH${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
        fi
        exec "$NESTED_BINARY" --winit
    ) >"$NESTED_LOG" 2>&1 &
    NESTED_WM_PID=$!

    # 100 ms polling ticks; same 60 s overall budget as the former 60
    # attempts of `sleep 1`.
    attempt=0
    until nested_log_has "minde scheme layer loaded"; do
        attempt=$((attempt + 1))
        [ "$attempt" -le 600 ] || return 1
        kill -0 "$NESTED_WM_PID" 2>/dev/null || return 1
        sleep 0.1
    done

    # Same 20 s overall budget as the former 20 attempts of `sleep 1`.
    attempt=0
    while :; do
        for socket in "$NESTED_RT"/wayland-*; do
            if [ -S "$socket" ]; then
                NESTED_WAYLAND_DISPLAY=${socket##*/}
                export NESTED_WAYLAND_DISPLAY
                return 0
            fi
        done
        attempt=$((attempt + 1))
        [ "$attempt" -le 200 ] || return 1
        sleep 0.1
    done
}

nested_wayland() ( nested_wayland_exec "$@"; )

# Use this form for background clients so $! is the executable's PID.
nested_wayland_exec() {
    exec env DISPLAY= WAYLAND_DISPLAY="$NESTED_WAYLAND_DISPLAY" \
        XDG_RUNTIME_DIR="$NESTED_RT" HOME="$NESTED_RT/home" \
        XDG_CACHE_HOME="$NESTED_RT/cache" XDG_CONFIG_HOME="$NESTED_RT/config" \
        XDG_STATE_HOME="$NESTED_RT/state" "$@"
}

nested_window_count() {
    scripts/mindectl query state --json | jq '[.groups[].window_count] | add // 0'
}

nested_wait_for_window_after() {
    previous=$1
    limit=$2
    attempt=0
    while [ "$attempt" -lt "$limit" ]; do
        current=$(nested_window_count 2>/dev/null || printf '0')
        [ "$current" -gt "$previous" ] && return 0
        kill -0 "$NESTED_WM_PID" 2>/dev/null || return 1
        attempt=$((attempt + 1))
        sleep 1
    done
    return 1
}

nested_wait_for_log_after() {
    line=$1
    pattern=$2
    limit=$3
    attempt=0
    while [ "$attempt" -lt "$limit" ]; do
        tail -n "+$line" "$NESTED_LOG" | nested_plain_log | grep -aq "$pattern" && return 0
        kill -0 "$NESTED_WM_PID" 2>/dev/null || return 1
        attempt=$((attempt + 1))
        sleep 1
    done
    return 1
}

nested_capture() {
    output=$1
    if command -v import >/dev/null 2>&1; then
        DISPLAY=$NESTED_DISPLAY import -window root "$output" 2>/dev/null || true
    fi
}
