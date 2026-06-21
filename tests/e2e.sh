#!/bin/sh
# Headless end-to-end test of the nested (winit) backend under Xvfb:
# starts the compositor with the repo config, drives it with XTEST key
# events, and asserts pass/fail from the debug log plus screenshot
# non-emptiness. Screenshots are left in $E2E_OUT (default /tmp/minde-e2e)
# for eyeballing.
#
# Run inside the dev shell with the extra tools:
#   guix shell -m manifest.scm xorg-server xdotool imagemagick foot -- tests/e2e.sh
#
# NOTE: this cannot exercise the udev/DRM backend (no seat/DRM headless);
# TTY changes still need a live `./debug-tty.sh` run from a spare VT.
set -eu
cd "$(dirname "$0")/.."

OUT="${E2E_OUT:-/tmp/minde-e2e}"
RT=/tmp/minde-e2e-rt
LOG="$OUT/e2e.log"
mkdir -p "$OUT" "$RT"
chmod 700 "$RT"

fail() { echo "FAIL - $1"; [ -f "$LOG" ] && tail -5 "$LOG"; kill "${WM_PID:-0}" "${XVFB_PID:-0}" 2>/dev/null || true; exit 1; }
ok() { echo "ok - $1"; }
# tracing writes ANSI color codes even into the redirected log; strip
# them before matching or patterns like "cmd=foot" never hit.
ESC=$(printf '\033')
loggrep() { sed "s/$ESC\[[0-9;]*m//g" "$LOG" | grep -aq "$1"; }

Xvfb :97 -screen 0 1280x800x24 &
XVFB_PID=$!
sleep 2

export DISPLAY=:97 XDG_RUNTIME_DIR="$RT" XKB_DEFAULT_LAYOUT=us
export LD_LIBRARY_PATH="${GUIX_ENVIRONMENT:-/nonexistent}/lib"
# Note "scheme" is its own tracing target (wm-log); filtering only
# minde=debug would hide every Scheme-side line the asserts rely on.
export RUST_LOG=minde=debug,scheme=info
unset WAYLAND_DISPLAY

cargo build 2>/dev/null || fail "cargo build"
MINDE_INIT="$PWD/scheme/init.scm" MINDE_SCHEME_DIR="$PWD/scheme" \
  ./target/debug/minde --winit > "$LOG" 2>&1 &
WM_PID=$!

# Startup can take a while when Guile recompiles modules; wait up to 60s.
i=0
until loggrep "minde scheme layer loaded"; do
  i=$((i + 1))
  [ "$i" -le 60 ] || fail "scheme layer did not load within 60s"
  kill -0 "$WM_PID" 2>/dev/null || fail "compositor died at startup"
  sleep 1
done
sleep 1
ok "compositor up, scheme layer loaded"

xdotool search --name Smithay windowfocus || fail "focus nested window"

# Native prompt: Print r, type, TAB-complete, Return -> spawns foot.
xdotool key Print; sleep 0.3
xdotool key r; sleep 0.5
import -window root "$OUT/prompt.png"
xdotool type --delay 60 "foo"; sleep 0.3
xdotool key Tab; sleep 0.3
xdotool key Return
sleep 4
loggrep 'wm-spawn cmd=foot' || fail "run prompt did not spawn TAB-completed foot"
loggrep "wm-on-window-map\|window map" || true
xdotool key Print; sleep 0.2; xdotool key y; sleep 0.5
import -window root "$OUT/info.png"
ok "run prompt: type, complete, spawn"

# Reload: Print R must succeed (echoes 'reloaded').
xdotool key Print; sleep 0.2; xdotool key shift+r; sleep 2
import -window root "$OUT/reload.png"
loggrep "reloaded " || fail "Print R reload did not report success"
ok "Print R reload"

# Splits and window cycling don't error.
xdotool key Print; sleep 0.2; xdotool key v; sleep 0.5
xdotool key Print; sleep 0.2; xdotool key Print; sleep 0.5
loggrep "error in keybinding" && fail "a keybinding errored (see log)"
ok "split + Print Print cycle without errors"

# iresize mode: Print s, nudge the divider, exit -- must not error.
xdotool key Print; sleep 0.2; xdotool key s; sleep 0.3
xdotool key Down; sleep 0.2; xdotool key Down; sleep 0.2
xdotool key Return; sleep 0.5
loggrep "error in keybinding" && fail "iresize errored (see log)"
ok "iresize mode"

# Layouts + gaps via the REPL socket, with a log marker to assert on.
scripts/minde-cmd '(begin
  (use-modules (minde layouts) (minde frames))
  (apply-layout! "grid4")
  (set-gaps! 8 8)
  (wm-log "e2e-layout-and-gaps-ok"))' >/dev/null 2>&1 || true
sleep 1
loggrep "e2e-layout-and-gaps-ok" || fail "layout/gaps REPL round-trip"
loggrep "error in keybinding" && fail "layout/gaps errored (see log)"
import -window root "$OUT/layout.png"
ok "grid4 layout + gaps applied via REPL"

# Screenshots must not be blank (a uniform 1280x800 PNG is ~1KB).
for shot in prompt reload; do
  size=$(wc -c < "$OUT/$shot.png")
  [ "$size" -gt 2000 ] || fail "$shot.png looks blank ($size bytes)"
done
ok "screenshots non-blank (in $OUT)"

kill "$WM_PID" "$XVFB_PID" 2>/dev/null || true
echo "e2e: all checks passed"
