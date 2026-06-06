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
   ;; misc
   "git"))
