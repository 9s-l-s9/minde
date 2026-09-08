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

    if [ -z "${MINDE_NESTED_PACKAGE:-}" ]; then
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

    Xvfb "$NESTED_DISPLAY" -screen 0 1280x800x24 >"$NESTED_OUT/xvfb.log" 2>&1 &
    NESTED_XVFB_PID=$!
    # Poll for Xvfb readiness instead of a fixed 2 s sleep; same 2 s overall
    # budget, checked every 100 ms so a fast start doesn't pay the whole
    # wait.
    attempt=0
    while :; do
        if command -v xdpyinfo >/dev/null 2>&1; then
            DISPLAY="$NESTED_DISPLAY" xdpyinfo >/dev/null 2>&1 && break
        elif [ -S "/tmp/.X11-unix/X${NESTED_DISPLAY#:}" ]; then
            break
        fi
        attempt=$((attempt + 1))
        [ "$attempt" -le 20 ] || return 1
        kill -0 "$NESTED_XVFB_PID" 2>/dev/null || return 1
        sleep 0.1
    done

    export DISPLAY="$NESTED_DISPLAY"
    export XDG_RUNTIME_DIR="$NESTED_RT"
    # A deterministic keymap regardless of the host session; a gate that
    # needs several layout groups sets NESTED_XKB_LAYOUT/NESTED_XKB_VARIANT
    # (tests/keyboard-layout-e2e.sh).
    export XKB_DEFAULT_LAYOUT="${NESTED_XKB_LAYOUT:-us}"
    unset XKB_DEFAULT_VARIANT XKB_DEFAULT_OPTIONS XKB_DEFAULT_MODEL XKB_DEFAULT_RULES
    [ -z "${NESTED_XKB_VARIANT:-}" ] || export XKB_DEFAULT_VARIANT="$NESTED_XKB_VARIANT"
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
    (
        if [ -n "${MINDE_NESTED_PACKAGE:-}" ]; then
            # Test the packaged runtime and its own library search paths.
            unset LD_LIBRARY_PATH
            export GUILE_LOAD_PATH="$MINDE_NESTED_PACKAGE/share/guile/site/3.0"
            export GUILE_LOAD_COMPILED_PATH="$MINDE_NESTED_PACKAGE/lib/guile/3.0/site-ccache"
            export GUILE_AUTO_COMPILE=0
        fi
        # A host autocompile cache can hide a missing packaged init.go.
        export XDG_CACHE_HOME="$NESTED_RT/cache"
        export MINDE_INIT="${MINDE_NESTED_INIT:-$NESTED_SCHEME/init.scm}"
        export MINDE_SCHEME_DIR="$NESTED_SCHEME"
        export MINDE_CONFIG="${MINDE_NESTED_CONFIG:-$PWD/tests/e2e-config.scm}"
        export MINDE_RULES_FILE="$NESTED_OUT/rules.scm"
        export MINDE_LAYOUTS_FILE="$NESTED_OUT/layouts.scm"
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
