#!/bin/sh
# Debug run of minde's DRM/TTY backend. Run from a text console
# (Ctrl+Alt+F3). Uses the session's XKB settings and writes the log to
# the user's XDG state directory.
#
#   ./debug-tty.sh
#
# Then wiggle the mouse, press some keys, wait ~15s. To get out:
# Ctrl+Alt+BackSpace (unless rebound/disabled), or switch to another VT.
# Emergency exit ends the session without asking applications to save.
cd "$(dirname "$0")" || exit 1
# The single-quoted program is intentionally evaluated by the inner Guix
# shell, where HOME/XDG_STATE_HOME have their final session values.
# shellcheck disable=SC2016
exec guix shell -m manifest.scm -- sh -c '
  export RUST_LOG=minde=debug,smithay=info
  state_dir="${XDG_STATE_HOME:-$HOME/.local/state}/minde"
  mkdir -p "$state_dir"
  echo "minde debug log: $state_dir/tty.log"
  cargo run -- --tty > "$state_dir/tty.log" 2>&1
'
