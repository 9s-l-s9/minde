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
  MINDE_RULES_FILE="$OUT/rules.scm" MINDE_LAYOUTS_FILE="$OUT/layouts.scm" \
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

# Floating: F floats the focused window (geometry lands in %floating),
# F again unfloats. Assert via a REPL log marker; the geometry must
# persist across a focus cycle.
xdotool key Print; sleep 0.2; xdotool key shift+f; sleep 0.5
scripts/minde-cmd '(begin
  (use-modules (minde frames))
  (wm-log (format #f "e2e-float ~s ~s"
                  (window-floating? (focused-window-id))
                  (float-geometry (focused-window-id)))))' >/dev/null 2>&1 || true
sleep 1
loggrep "e2e-float #t (" || fail "float-this did not float the window"
xdotool key Print; sleep 0.2; xdotool key f; sleep 0.3
scripts/minde-cmd '(begin
  (use-modules (minde frames))
  (wm-log (format #f "e2e-float-still ~s"
                  (pair? (group-floats (current-group))))))' >/dev/null 2>&1 || true
sleep 1
loggrep "e2e-float-still #t" || fail "float lost across a focus cycle"
import -window root "$OUT/float.png"
# Unfloat via the REPL (key-cycling back onto the float would depend on
# how many windows earlier blocks left open).
scripts/minde-cmd '(begin
  (use-modules (minde frames))
  (unfloat-window! (car (group-floats (current-group))))
  (wm-log (format #f "e2e-unfloat ~s"
                  (null? (group-floats (current-group))))))' >/dev/null 2>&1 || true
sleep 1
loggrep "e2e-unfloat #t" || fail "unfloat did not clear the float list"
loggrep "error in keybinding" && fail "float keys errored (see log)"
ok "float-this: float + persistence + unfloat"

# Menu: the windowlist is now a navigable menu; C-n + Return must focus
# a window without keybinding errors.
xdotool key Print; sleep 0.2; xdotool key l; sleep 0.5
import -window root "$OUT/menu.png"
xdotool key ctrl+n; sleep 0.2
xdotool key Return; sleep 0.5
loggrep "error in keybinding" && fail "menu windowlist errored (see log)"
ok "menu windowlist: open / navigate / select"

# Polish sprint: send-string / ratclick / idle / rename / ontop /
# flatten must all execute without erroring (send-string types into
# whatever is focused; we only assert the round-trip).
scripts/minde-cmd '(begin
  (use-modules (minde frames))
  (window-send-string "echo hi")
  (ratclick! 1)
  (rename-window! "e2e-renamed")
  (toggle-always-on-top!)
  (toggle-always-on-top!)
  (wm-log (format #f "e2e-polish idle=~a" (>= (idle-ms) 0))))' >/dev/null 2>&1 || true
sleep 1
loggrep "e2e-polish idle=#t" || fail "polish commands round-trip"
loggrep "error in keybinding" && fail "polish block errored (see log)"
ok "send-string / ratclick / rename / ontop / idle"

# Command mode: Print z, then prefix keys work bare until Return.
xdotool key Print; sleep 0.2; xdotool key z; sleep 0.3
xdotool key v; sleep 0.3          # bare vsplit
xdotool key c; sleep 0.3          # bare remove-split
xdotool key Return; sleep 0.3     # leave command mode
loggrep "error in keybinding" && fail "command mode errored (see log)"
ok "command mode: enter / bare keys / exit"

# Sprint 8: gnewbg (created, not switched), sticky follows a group
# switch, and the pull-from-windowlist menu -- all via REPL markers.
scripts/minde-cmd '(begin
  (use-modules (minde frames) (minde groups))
  (let ((before (current-group-name)))
    (gnewbg! " e2eBG ")
    (wm-log (format #f "e2e-gnewbg ~s ~s"
                    (and (member " e2eBG " (group-names)) #t)
                    (string=? (current-group-name) before)))))' >/dev/null 2>&1 || true
sleep 1
loggrep "e2e-gnewbg #t #t" || fail "gnewbg did not create in the background"
scripts/minde-cmd '(begin
  (use-modules (minde frames) (minde groups))
  (toggle-always-show!)
  (switch-to-group! " e2eBG ")
  (wm-log (format #f "e2e-sticky ~s"
                  (and (focused-window-id)
                       (group-has-window? " e2eBG " (focused-window-id))
                       #t)))
  (toggle-always-show!)
  (gother!))' >/dev/null 2>&1 || true
sleep 1
loggrep "e2e-sticky #t" || fail "sticky window did not follow the group switch"
# pull-from-windowlist: open the menu (Print M-p), take the first entry.
xdotool key Print; sleep 0.2; xdotool key alt+p; sleep 0.5
xdotool key Return; sleep 0.5
loggrep "error in keybinding" && fail "pull-from-windowlist errored (see log)"
ok "gnewbg / always-show / pull-from-windowlist"

# Sprint 9: fselect (overlays + digit jump), expose in/out, remember +
# place-existing round-trip.
xdotool key Print; sleep 0.2; xdotool key j; sleep 0.4
import -window root "$OUT/fselect.png"
xdotool key 0; sleep 0.4
xdotool key Print; sleep 0.2; xdotool key alt+e; sleep 0.5
import -window root "$OUT/expose.png"
xdotool key Escape; sleep 0.4
loggrep "error in keybinding" && fail "fselect/expose errored (see log)"
scripts/minde-cmd '(begin
  (use-modules (minde frames) (minde groups))
  (remember!)
  (place-existing-windows!)
  (forget!)
  (wm-log "e2e-remember-ok"))' >/dev/null 2>&1 || true
sleep 1
loggrep "e2e-remember-ok" || fail "remember/forget round-trip"
ok "fselect / expose / remember"

# Sprint 10: remapped keys + send-key through the real subr, which-key
# auto-echo, help prompts. A fresh Wayland client first: its app-id
# only arrives after map, via wm-on-window-title.
scripts/minde-cmd '(wm-spawn "foot")' >/dev/null 2>&1 || true
sleep 2
scripts/minde-cmd '(begin
  (use-modules (minde frames))
  (define-remapped-keys! (list (list ".*" (cons "C-F9" "Down"))))
  (wm-log (format #f "e2e-remap ~s" (remap-target "C-F9")))
  (wm-log (format #f "e2e-appids ~s"
                  (map (lambda (id) (window-app-id id)) (all-window-ids))))
  (send-key "Down")
  (wm-log "e2e-sendkey-ok"))' >/dev/null 2>&1 || true
sleep 1
loggrep 'e2e-remap "Down"' || fail "remap-target did not resolve"
# Titles/app-ids must arrive post-map via wm-on-window-title.
loggrep 'e2e-appids .*foot' || fail "app-ids never arrived (wm-on-window-title)"
loggrep "e2e-sendkey-ok" || fail "send-key errored"
# Exercise the live remap branch (consumed, synthesized, no errors),
# then drop the table again.
xdotool key ctrl+F9; sleep 0.3
scripts/minde-cmd '(begin
  (use-modules (minde frames))
  (unbind-remapped-keys!))' >/dev/null 2>&1 || true
# which-key auto-echo: arm the prefix, wait past the delay, screenshot.
scripts/minde-cmd '(which-key-mode!)' >/dev/null 2>&1 || true
xdotool key Print; sleep 1.6
import -window root "$OUT/whichkey.png"
xdotool key Escape; sleep 0.3
scripts/minde-cmd '(which-key-mode!)' >/dev/null 2>&1 || true
# Help prompts open and abort cleanly.
xdotool key Print; sleep 0.2; xdotool key F2; sleep 0.4
xdotool key Escape; sleep 0.3
xdotool key Print; sleep 0.2; xdotool key F5; sleep 0.4
xdotool key Escape; sleep 0.3
loggrep "error in keybinding" && fail "sprint-10 keys/help errored (see log)"
ok "remapped keys / send-key / which-key / help prompts"

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
