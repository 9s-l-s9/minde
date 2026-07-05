#!/bin/sh
# SPDX-License-Identifier: GPL-3.0-or-later
# Shared nested-compositor lifecycle used by application and soak scenarios.

nested_log_has() {
    sed "s/$(printf '\033')\[[0-9;]*m//g" "$NESTED_LOG" | grep -aq "$1"
}

nested_stop() {
    for pid in "${NESTED_WM_PID:-}" "${NESTED_XVFB_PID:-}"; do
        if [ -n "$pid" ]; then
            kill "$pid" 2>/dev/null || true
            wait "$pid" 2>/dev/null || true
        fi
    done
    if [ -n "${NESTED_RT:-}" ] && [ -d "$NESTED_RT" ]; then
        rm -rf "$NESTED_RT"
    fi
}

nested_start() {
    NESTED_OUT=$1
    NESTED_DISPLAY=$2
    NESTED_RT=$(mktemp -d /tmp/minde-nested-runtime.XXXXXX)
    NESTED_LOG="$NESTED_OUT/compositor.log"
    mkdir -p "$NESTED_OUT"
    mkdir -p "$NESTED_RT/home" "$NESTED_RT/cache" "$NESTED_RT/config" \
        "$NESTED_RT/state"
    chmod 700 "$NESTED_RT"
    : >"$NESTED_LOG"

    command -v cargo >/dev/null 2>&1 || {
        echo "error: cargo is required; enter the project Guix shell" >&2
        return 127
    }
    command -v Xvfb >/dev/null 2>&1 || {
        echo "error: Xvfb is required; add xorg-server" >&2
        return 127
    }
    command -v jq >/dev/null 2>&1 || {
        echo "error: jq is required for structured scenario assertions" >&2
        return 127
    }

    Xvfb "$NESTED_DISPLAY" -screen 0 1280x800x24 >"$NESTED_OUT/xvfb.log" 2>&1 &
    NESTED_XVFB_PID=$!
    sleep 2

    export DISPLAY="$NESTED_DISPLAY"
    export XDG_RUNTIME_DIR="$NESTED_RT"
    export XKB_DEFAULT_LAYOUT=us
    unset XKB_DEFAULT_VARIANT XKB_DEFAULT_OPTIONS XKB_DEFAULT_MODEL XKB_DEFAULT_RULES
    unset MINDE_REPL_STARTED WAYLAND_DISPLAY MINDE_FULL_KEYMAP \
        MINDE_E2E_LEGACY_KEYMAP
    export LD_LIBRARY_PATH="${GUIX_ENVIRONMENT:-/nonexistent}/lib${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
    export RUST_LOG=minde=debug,scheme=info

    cargo build >"$NESTED_OUT/build.log" 2>&1 || return 1
    MINDE_INIT="$PWD/scheme/init.scm" \
        MINDE_SCHEME_DIR="$PWD/scheme" \
        MINDE_CONFIG="${MINDE_NESTED_CONFIG:-$PWD/tests/e2e-config.scm}" \
        MINDE_RULES_FILE="$NESTED_OUT/rules.scm" \
        MINDE_LAYOUTS_FILE="$NESTED_OUT/layouts.scm" \
        ./target/debug/minde --winit >"$NESTED_LOG" 2>&1 &
    NESTED_WM_PID=$!

    attempt=0
    until nested_log_has "minde scheme layer loaded"; do
        attempt=$((attempt + 1))
        [ "$attempt" -le 60 ] || return 1
        kill -0 "$NESTED_WM_PID" 2>/dev/null || return 1
        sleep 1
    done

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
        [ "$attempt" -le 20 ] || return 1
        sleep 1
    done
}

nested_wayland() {
    env DISPLAY= WAYLAND_DISPLAY="$NESTED_WAYLAND_DISPLAY" \
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
        tail -n "+$line" "$NESTED_LOG" | grep -aq "$pattern" && return 0
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
