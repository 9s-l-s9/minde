#!/bin/sh
# Debug run of minde's DRM/TTY backend. Run from a text console
# (Ctrl+Alt+F3). Writes everything to tty.log in this directory.
#
#   ./debug-tty.sh
#
# Then wiggle the mouse, press some keys, wait ~15s. To get out:
# super+q, or Ctrl+Alt+F7 back to X, or from elsewhere: pkill minde.
cd "$(dirname "$0")"
exec guix shell -m manifest.scm -- sh -c '
  export XKB_DEFAULT_LAYOUT=de XKB_DEFAULT_VARIANT=bone
  export RUST_LOG=minde=debug,smithay=info
  cargo run -- --tty > tty.log 2>&1
'
