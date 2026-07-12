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
   "jq"
   "util-linux"
   "foot"
   "xterm"
   "wl-clipboard"
   "shellcheck"
   ;; used by the static/doc-drift gates; without them ./check cannot run
   ;; self-contained in this shell
   "ripgrep"
   "diffutils"
   ;; Release demonstration encoder; capture remains explicitly opt-in.
   "ffmpeg"
   ;; misc
   "git"))
