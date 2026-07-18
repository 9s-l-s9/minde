;; Development environment for the compositor.
;; Usage: guix shell -m manifest.scm
(specifications->manifest
 '(;; Rust toolchain
   "rust"
   "rust:cargo"
   "rust:tools"
   "gcc-toolchain"
   "pkg-config"
   ;; Guile (embedded via libguile)
   "guile"
   ;; Smithay native deps
   "wayland"
   "wayland-protocols"
   "libxkbcommon"
   "libinput-minimal"
   "eudev"
   "libseat"
   "mesa"
   "libglvnd"
   ;; X11 backend for running nested inside StumpWM.
   ;; winit dlopens these at runtime; they must also be on LD_LIBRARY_PATH
   ;; (see README: run with LD_LIBRARY_PATH=$GUIX_ENVIRONMENT/lib).
   "libx11"
   "libxcb"
   "libxi"
   "libxcursor"
   "libxrandr"
   ;; embedded X server for X11 clients (spawned from PATH at runtime;
   ;; also exercised by the e2e X11 block together with xterm)
   "xorg-server-xwayland"
   ;; Bounded local verification.  Large browser/toolkit application shards
   ;; remain opt-in so the normal development environment stays reasonable.
   "xorg-server"
   "xdotool"
   "imagemagick"
   ;; ext-image-copy-capture-v1 client used by the screen-capture e2e gate
   "grim"
   ;; wlr-screencopy-unstable-v1 client used by the screen-capture e2e gate
   "wf-recorder"
   "jq"
   "util-linux"
   "foot"
   "xterm"
   "wl-clipboard"
   ;; wlr-output-management client used by the output-management e2e gate
   "wlr-randr"
   ;; wayland-info (global enumeration) used by the clipboard and
   ;; foreign-toplevel e2e gates to assert the manager globals are advertised
   "wayland-utils"
   ;; swayidle: ext-idle-notify-v1 client used by the idle e2e gate to
   ;; assert the compositor actually fires an idle notification
   "swayidle"
   "shellcheck"
   ;; used by the static/doc-drift gates; without them ./check cannot run
   ;; self-contained in this shell
   "ripgrep"
   "diffutils"
   ;; Release demonstration encoder; capture remains explicitly opt-in.
   "ffmpeg"
   ;; misc
   "git"))
