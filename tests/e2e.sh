#!/bin/sh
# Headless end-to-end test of the nested (winit) backend under Xvfb:
# starts the compositor with the repo config, drives it with XTEST key
# events, and asserts pass/fail from the debug log plus screenshot
# non-emptiness. Screenshots are left in $E2E_OUT (default /tmp/minde-e2e)
# for eyeballing.
#
# Run inside the dev shell with the extra tools:
#   guix shell -m manifest.scm xorg-server xdotool imagemagick foot xterm -- tests/e2e.sh
# (xterm exercises the Xwayland block; it is skipped when absent.)
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

# Navigation sprint: directional focus, last-window toggle, select by
# number, windows echo -- none may error.
xdotool key Print; sleep 0.2; xdotool key Down; sleep 0.3
xdotool key Print; sleep 0.2; xdotool key Tab; sleep 0.3
xdotool key Print; sleep 0.2; xdotool key 0; sleep 0.3
xdotool key Print; sleep 0.2; xdotool key shift+w; sleep 0.5
import -window root "$OUT/windows-echo.png"
loggrep "error in keybinding" && fail "navigation keys errored (see log)"
ok "navigation: arrows / Tab / 0 / W"

# Sprint 2: which-key, describe-key, eval prompt, marks -- no errors.
xdotool key Print; sleep 0.2; xdotool key question; sleep 0.3
xdotool key Escape; sleep 0.2 # leave the armed prefix
xdotool key Print; sleep 0.2; xdotool key F1; sleep 0.2; xdotool key v; sleep 0.3
xdotool key Print; sleep 0.2; xdotool key colon; sleep 0.3
xdotool type --delay 40 "(+ 1 2)"; sleep 0.2; xdotool key Return; sleep 0.5
import -window root "$OUT/eval.png"
xdotool key Print; sleep 0.2; xdotool key x; sleep 0.3
loggrep "error in keybinding" && fail "sprint-2 keys errored (see log)"
ok "which-key / describe-key / eval / marks"

# Sprint 3: timer subr, fullscreen toggle, banish, frame flash, clipboard.
scripts/minde-cmd '(wm-run-after 100 (lambda () (wm-log "e2e-timer-ok")))' >/dev/null 2>&1 || true
sleep 1
loggrep "e2e-timer-ok" || fail "wm-run-after timer round-trip"
xdotool key Print; sleep 0.2; xdotool key alt+f; sleep 0.5
xdotool key Print; sleep 0.2; xdotool key alt+f; sleep 0.5
xdotool key Print; sleep 0.2; xdotool key shift+b; sleep 0.3
xdotool key Print; sleep 0.2; xdotool key ctrl+c; sleep 0.6
loggrep "error in keybinding" && fail "sprint-3 keys errored (see log)"
loggrep "error in timer" && fail "a timer thunk errored (see log)"
# Clipboard: own the selection compositor-side, paste it into the eval
# prompt with C-y, evaluate.
scripts/minde-cmd '(set-clipboard! "(+ 2 3)")' >/dev/null 2>&1 || true
sleep 0.5
xdotool key Print; sleep 0.2; xdotool key colon; sleep 0.3
xdotool key ctrl+y; sleep 0.5
xdotool key Return; sleep 0.5
import -window root "$OUT/paste.png"
loggrep "error in keybinding" && fail "clipboard paste errored (see log)"
ok "timer / fullscreen / banish / flash / clipboard paste"

# Multi-head keys: with the single nested head they must echo "only one
# head" rather than erroring; wm-outputs must return the winit head.
xdotool key Print; sleep 0.2; xdotool key shift+s; sleep 0.3
xdotool key Print; sleep 0.2; xdotool key alt+s; sleep 0.3
loggrep "error in keybinding" && fail "head keys errored (see log)"
scripts/minde-cmd '(wm-log (format #f "e2e-outputs ~s" (wm-outputs)))' >/dev/null 2>&1 || true
sleep 1
loggrep "e2e-outputs ((0 " || fail "wm-outputs did not report the winit head"
ok "head keys + wm-outputs (single head)"

# Xwayland: an X11 client (xterm) must map as a managed window, with the
# 'new-window hook seeing its class as the app-id. Skips cleanly when
# Xwayland/xterm aren't in the shell.
if command -v Xwayland >/dev/null 2>&1 && command -v xterm >/dev/null 2>&1; then
  scripts/minde-cmd '(begin
    (use-modules (minde hooks))
    (add-hook!* (quote new-window)
      (lambda (id title app-id) (wm-log (format #f "e2e-x11-map ~a" app-id))))
    (wm-spawn "xterm"))' >/dev/null 2>&1 || true
  i=0
  until loggrep "e2e-x11-map.*[Xx][Tt]erm"; do
    i=$((i + 1))
    [ "$i" -le 15 ] || fail "xterm did not map via Xwayland within 15s"
    sleep 1
  done
  import -window root "$OUT/x11.png"
  loggrep "error in keybinding" && fail "x11 block errored (see log)"
  ok "xwayland: xterm mapped as a managed window"
else
  echo "skip - Xwayland/xterm not in the shell; X11 e2e block skipped"
fi

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
