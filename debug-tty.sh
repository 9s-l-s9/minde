#!/bin/sh
# Debug run of minde's DRM/TTY backend. Run from a text console
# (Ctrl+Alt+F3). Writes to the user's XDG state directory so a debug run
# never dirties the source tree.
#
#   ./debug-tty.sh
#
# Then wiggle the mouse, press some keys, wait ~15s. To get out:
# super+q, or Ctrl+Alt+F7 back to X, or from elsewhere: pkill minde.
cd "$(dirname "$0")" || exit 1
# The single-quoted program is intentionally evaluated by the inner Guix
# shell, where HOME/XDG_STATE_HOME have their final session values.
# shellcheck disable=SC2016
exec guix shell -m manifest.scm -- sh -c '
  export XKB_DEFAULT_LAYOUT=de XKB_DEFAULT_VARIANT=bone
  export RUST_LOG=minde=debug,smithay=info
  state_dir="${XDG_STATE_HOME:-$HOME/.local/state}/minde"
  mkdir -p "$state_dir"
  echo "minde debug log: $state_dir/tty.log"
  cargo run -- --tty > "$state_dir/tty.log" 2>&1
'
